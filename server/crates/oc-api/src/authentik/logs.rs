//! An authentik event as a log (`source:authentik`): Datadog's standard attributes (`evt.name`, `evt.outcome`,
//! `usr.*`, `network.client.*`, `http.*`), the event's own fields under `authentik.*`, its time as `date`, and an
//! identity from the event's id, so reading an event again rewrites the same row.

use serde_json::{json, Map, Value};

/// Actions whose events are errors (and failures).
const ERRORS: &[&str] = &["login_failed", "suspicious_request", "system_exception", "system_task_exception", "policy_exception", "property_mapping_exception", "configuration_error"];
/// Actions whose events are warnings.
const WARNINGS: &[&str] = &["configuration_warning", "access_request_denied", "impersonation_started"];

fn text<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(*k)).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `access_request_denied` → `Access request denied`.
fn words(action: &str) -> String {
    let spaced = action.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// What the event touched: an access request's target, the application opened, or the model written.
fn subject(context: &Value) -> Option<&str> {
    text(context, &["target", "name"]).or_else(|| text(context, &["application", "name"])).or_else(|| text(context, &["authorized_application", "name"])).or_else(|| text(context, &["model", "name"]))
}

fn message(action: &str, user: Option<&str>, context: &Value) -> String {
    let who = user.unwrap_or("someone");
    match action {
        "login" => match text(context, &["auth_method"]) {
            Some(method) => format!("{who} logged in ({method})"),
            None => format!("{who} logged in"),
        },
        "login_failed" => format!("Login failed for {}", text(context, &["username"]).unwrap_or(who)),
        "logout" => format!("{who} logged out"),
        "authorize_application" => format!("{who} opened {}", subject(context).unwrap_or("an application")),
        "model_created" | "model_updated" | "model_deleted" => {
            let verb = action.trim_start_matches("model_");
            let model = text(context, &["model", "model_name"]).unwrap_or("object");
            match text(context, &["model", "name"]) {
                Some(name) => format!("{who} {verb} {model} {name}"),
                None => format!("{who} {verb} {model}"),
            }
        }
        _ if action.starts_with("access_request_") => match (subject(context), user) {
            (Some(target), Some(u)) => format!("{} for {target} ({u})", words(action)),
            (Some(target), None) => format!("{} for {target}", words(action)),
            (None, Some(u)) => format!("{} ({u})", words(action)),
            (None, None) => words(action),
        },
        _ => match (text(context, &["message"]), user) {
            (Some(detail), _) => format!("{}: {detail}", words(action)),
            (None, Some(u)) => format!("{} ({u})", words(action)),
            (None, None) => words(action),
        },
    }
}

/// FNV-1a of the event's id: the random bits of its log identity.
fn seed(pk: &str) -> u128 {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let high = fnv(0xcbf2_9ce4_8422_2325, pk.as_bytes());
    ((high as u128) << 64) | fnv(high, b"authentik") as u128
}

/// The log of an authentik event, from the instance at `host`.
pub fn log_of(event: &Value, host: &str) -> Value {
    let action = event["action"].as_str().unwrap_or("custom");
    let context = event.get("context").cloned().unwrap_or(Value::Null);
    let user = text(event, &["user", "username"]);
    let created = event["created"].as_str().unwrap_or_default();
    let at = chrono::DateTime::parse_from_rfc3339(created).map(|t| t.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    let pk = event["pk"].as_str().unwrap_or_default();
    let status = if ERRORS.contains(&action) {
        "error"
    } else if WARNINGS.contains(&action) {
        "warn"
    } else {
        "info"
    };
    let failure = status == "error" || action == "access_request_denied";
    let mut log = json!({
        "date": created,
        "message": message(action, user, &context),
        "status": status,
        "service": "authentik",
        "ddsource": "authentik",
        "host": host,
        "evt": { "name": action, "outcome": if failure { "failure" } else { "success" } },
        "_oc_ingest": { "id": crate::log_normalize::uuid_v7_seeded(at, seed(pk)) },
    });
    if let Some(name) = user {
        let mut usr = Map::new();
        if let Some(id) = event["user"].get("pk").filter(|v| !v.is_null()) {
            usr.insert("id".into(), Value::String(id.as_str().map_or_else(|| id.to_string(), str::to_string)));
        }
        usr.insert("name".into(), Value::String(name.to_string()));
        if let Some(email) = text(event, &["user", "email"]) {
            usr.insert("email".into(), Value::String(email.to_string()));
        }
        log["usr"] = Value::Object(usr);
    }
    if let Some(ip) = event["client_ip"].as_str().filter(|s| !s.is_empty()) {
        let mut client = json!({ "ip": ip });
        if let Some(geo) = context.get("geo").and_then(Value::as_object) {
            let mut geoip = Map::new();
            if let Some(c) = geo.get("country").and_then(Value::as_str) {
                geoip.insert("country".into(), json!({ "iso_code": c }));
            }
            if let Some(c) = geo.get("city").and_then(Value::as_str) {
                geoip.insert("city".into(), json!({ "name": c }));
            }
            if let Some(c) = geo.get("continent").and_then(Value::as_str) {
                geoip.insert("continent".into(), json!({ "code": c }));
            }
            if let (Some(lat), Some(lon)) = (geo.get("lat").and_then(Value::as_f64), geo.get("long").and_then(Value::as_f64)) {
                geoip.insert("location".into(), json!({ "latitude": lat, "longitude": lon }));
            }
            client["geoip"] = Value::Object(geoip);
        }
        if let Some(asn) = context.get("asn") {
            client["asn"] = json!({ "number": asn["asn"], "organization": asn["as_org"] });
        }
        log["network"] = json!({ "client": client });
    }
    if let Some(req) = context.get("http_request").filter(|r| r.is_object()) {
        let mut http = Map::new();
        if let Some(m) = req["method"].as_str() {
            http.insert("method".into(), Value::String(m.to_string()));
        }
        if let Some(p) = req["path"].as_str() {
            http.insert("url_details".into(), json!({ "path": p }));
        }
        if let Some(ua) = req["user_agent"].as_str() {
            http.insert("useragent".into(), Value::String(ua.to_string()));
        }
        log["http"] = Value::Object(http);
    }
    let mut own = json!({ "event_id": pk, "app": event["app"] });
    if let Some(brand) = text(event, &["brand", "name"]) {
        own["brand"] = Value::String(brand.to_string());
    }
    if let Some(app) = text(&context, &["authorized_application", "name"]) {
        own["application"] = Value::String(app.to_string());
    }
    if let Value::Object(mut rest) = context {
        for k in ["http_request", "geo", "asn"] {
            rest.remove(k);
        }
        own["context"] = Value::Object(rest);
    }
    log["authentik"] = own;
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentik::fixtures;
    use serde_json::json;

    #[test]
    fn a_login_is_an_info_log_with_the_standard_attributes() {
        let log = log_of(&fixtures::login("0b6c5d1e-8f2a-4c3b-9d7e-1a2b3c4d5e6f", "2026-11-06T10:00:00.123456Z"), "portal.example.com");
        assert_eq!(log["message"], "jordan.patel logged in (password)");
        assert_eq!((log["status"].as_str(), log["service"].as_str(), log["ddsource"].as_str(), log["host"].as_str()), (Some("info"), Some("authentik"), Some("authentik"), Some("portal.example.com")));
        assert_eq!(log["date"], "2026-11-06T10:00:00.123456Z");
        assert_eq!(log["evt"], json!({ "name": "login", "outcome": "success" }));
        assert_eq!(log["usr"], json!({ "id": "42", "name": "jordan.patel", "email": "jordan.patel@example.com" }));
        assert_eq!(log["network"]["client"]["ip"], "203.0.113.24");
        assert_eq!(log["network"]["client"]["geoip"]["country"]["iso_code"], "RO");
        assert_eq!(log["network"]["client"]["geoip"]["city"]["name"], "Bucharest");
        assert_eq!(log["network"]["client"]["asn"], json!({ "number": 64500, "organization": "Example ISP" }));
        assert_eq!((log["http"]["method"].as_str(), log["http"]["url_details"]["path"].as_str(), log["http"]["useragent"].as_str()), (Some("POST"), Some("/api/v3/flows/executor/default-authentication-flow/"), Some("Mozilla/5.0")));
        assert_eq!(log["authentik"]["event_id"], "0b6c5d1e-8f2a-4c3b-9d7e-1a2b3c4d5e6f");
        assert_eq!(log["authentik"]["brand"], "Example portal");
        assert_eq!(log["authentik"]["context"]["auth_method"], "password");
        assert!(log["authentik"]["context"].get("http_request").is_none(), "the request is under http.*");
    }

    #[test]
    fn failures_and_exceptions_are_errors_and_denials_warnings() {
        let failed = log_of(&fixtures::event("1", "login_failed", "2026-11-06T10:00:00Z", json!({ "username": "admin", "stage": "default-authentication-password" })), "portal.example.com");
        assert_eq!((failed["status"].as_str(), failed["evt"]["outcome"].as_str()), (Some("error"), Some("failure")));
        assert_eq!(failed["message"], "Login failed for admin");
        let exception = log_of(&fixtures::event("2", "system_exception", "2026-11-06T10:00:00Z", json!({ "message": "connection refused" })), "portal.example.com");
        assert_eq!((exception["status"].as_str(), exception["message"].as_str()), (Some("error"), Some("System exception: connection refused")));
        let denied = log_of(&fixtures::event("3", "access_request_denied", "2026-11-06T10:00:00Z", json!({ "target": { "name": "Grafana dev" } })), "portal.example.com");
        assert_eq!((denied["status"].as_str(), denied["message"].as_str()), (Some("warn"), Some("Access request denied for Grafana dev (jordan.patel)")));
    }

    #[test]
    fn applications_models_and_access_requests_name_what_they_touched() {
        let opened = log_of(&fixtures::event("4", "authorize_application", "2026-11-06T10:00:00Z", json!({ "authorized_application": { "name": "ObserveCraft dev", "pk": "a1" } })), "portal.example.com");
        assert_eq!(opened["message"], "jordan.patel opened ObserveCraft dev");
        assert_eq!(opened["authentik"]["application"], "ObserveCraft dev");
        let created = log_of(&fixtures::event("5", "model_created", "2026-11-06T10:00:00Z", json!({ "model": { "app": "authentik_core", "model_name": "user", "name": "sam.lee", "pk": 77 } })), "portal.example.com");
        assert_eq!(created["message"], "jordan.patel created user sam.lee");
        let requested = log_of(&fixtures::event("6", "access_request_created", "2026-11-06T10:00:00Z", json!({ "target": { "name": "Argo CD dev" } })), "portal.example.com");
        assert_eq!((requested["status"].as_str(), requested["message"].as_str()), (Some("info"), Some("Access request created for Argo CD dev (jordan.patel)")));
        let unknown = log_of(&fixtures::event("7", "review_overdue", "2026-11-06T10:00:00Z", json!({})), "portal.example.com");
        assert_eq!(unknown["message"], "Review overdue (jordan.patel)");
    }

    #[test]
    fn an_events_identity_is_stable_and_timed_at_the_event() {
        let e = fixtures::login("0b6c5d1e-8f2a-4c3b-9d7e-1a2b3c4d5e6f", "2026-11-06T10:00:00.123456Z");
        let (a, b) = (log_of(&e, "portal.example.com"), log_of(&e, "portal.example.com"));
        let id = a["_oc_ingest"]["id"].as_str().unwrap();
        assert_eq!(id, b["_oc_ingest"]["id"].as_str().unwrap(), "the same event, the same row");
        assert_eq!(&id[14..15], "7", "a UUIDv7");
        let ms = i64::from_str_radix(&id.replace('-', "")[..12], 16).unwrap();
        assert_eq!(ms, chrono::DateTime::parse_from_rfc3339("2026-11-06T10:00:00.123Z").unwrap().timestamp_millis());
        let other = log_of(&fixtures::login("1b6c5d1e-8f2a-4c3b-9d7e-1a2b3c4d5e6f", "2026-11-06T10:00:00.123456Z"), "portal.example.com");
        assert_ne!(other["_oc_ingest"]["id"], a["_oc_ingest"]["id"]);
    }

    #[test]
    fn events_without_a_user_still_read() {
        let mut e = fixtures::event("8", "system_task_exception", "2026-11-06T10:00:00Z", json!({ "message": "outpost unreachable" }));
        e["user"] = json!({});
        e["client_ip"] = serde_json::Value::Null;
        let log = log_of(&e, "portal.example.com");
        assert_eq!(log["message"], "System task exception: outpost unreachable");
        assert!(log.get("usr").is_none() && log.get("network").is_none());
    }
}
