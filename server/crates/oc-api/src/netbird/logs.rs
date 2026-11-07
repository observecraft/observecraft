//! A NetBird audit event as a log (`source:netbird`): Datadog's standard attributes (`evt.name` is the event's
//! activity code, `evt.outcome`, `usr.*` the initiator), the event's own fields under `netbird.*`, its time as `date`,
//! and an identity from the instance and the event's id, so reading an event again rewrites the same row.

use serde_json::{json, Map, Value};

/// Activity codes whose events are errors (and failures): a setup key used more often than allowed, a token replayed.
const ERRORS: &[&str] = &["setupkey.overuse", "mcp.session.replay"];
/// Activity codes whose events are warnings: access taken away or expired.
const WARNINGS: &[&str] = &["user.block", "user.reject", "peer.login.expire", "peer.approval.revoke", "setupkey.revoke", "account.delete", "service.peer.expose.expire"];

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// What the event touched, as NetBird names it in the event's metadata (a peer, user, policy, route, key, group).
fn subject(meta: &Value) -> Option<&str> {
    ["name", "peer_name", "email", "fqdn", "group", "key_name"].iter().find_map(|k| text(meta, k))
}

/// Who did it: the initiator's name, else its email; system events (expirations, overuse) have neither.
fn initiator(event: &Value) -> Option<&str> {
    text(event, "initiator_name").or_else(|| text(event, "initiator_email"))
}

fn message(event: &Value, code: &str) -> String {
    let activity = text(event, "activity").map(str::to_string).unwrap_or_else(|| code.to_string());
    let meta = event.get("meta").unwrap_or(&Value::Null);
    let mut out = match subject(meta) {
        Some(s) => format!("{activity}: {s}"),
        None => activity,
    };
    if let Some(who) = initiator(event) {
        out.push_str(&format!(" (by {who})"));
    }
    out
}

/// FNV-1a of the instance and the event's id: the random bits of its log identity (ids are per management server).
fn seed(host: &str, id: &str) -> u128 {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let high = fnv(fnv(0xcbf2_9ce4_8422_2325, host.as_bytes()), id.as_bytes());
    ((high as u128) << 64) | fnv(high, b"netbird") as u128
}

/// An event's id as text (NetBird answers a number).
pub fn event_id(event: &Value) -> String {
    match &event["id"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The log of a NetBird audit event, from the management server at `host`.
pub fn log_of(event: &Value, host: &str) -> Value {
    let code = text(event, "activity_code").unwrap_or("unknown");
    let timestamp = event["timestamp"].as_str().unwrap_or_default();
    let at = chrono::DateTime::parse_from_rfc3339(timestamp).map(|t| t.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    let id = event_id(event);
    let status = if ERRORS.contains(&code) {
        "error"
    } else if WARNINGS.contains(&code) {
        "warn"
    } else {
        "info"
    };
    let mut log = json!({
        "date": timestamp,
        "message": message(event, code),
        "status": status,
        "service": "netbird",
        "ddsource": "netbird",
        "host": host,
        "evt": { "name": code, "outcome": if status == "error" { "failure" } else { "success" } },
        "_oc_ingest": { "id": crate::log_normalize::uuid_v7_seeded(at, seed(host, &id)) },
    });
    let mut usr = Map::new();
    for (key, field) in [("id", "initiator_id"), ("name", "initiator_name"), ("email", "initiator_email")] {
        if let Some(v) = text(event, field) {
            usr.insert(key.into(), Value::String(v.to_string()));
        }
    }
    if !usr.is_empty() {
        log["usr"] = Value::Object(usr);
    }
    let mut own = json!({
        "event_id": id,
        // The code's first segment: peer, user, policy, route, setupkey, group, account, dns, network, …
        "category": code.split('.').next().unwrap_or(code),
    });
    if let Some(a) = text(event, "activity") {
        own["activity"] = Value::String(a.to_string());
    }
    if let Some(t) = text(event, "target_id") {
        own["target_id"] = Value::String(t.to_string());
    }
    if let Some(s) = subject(event.get("meta").unwrap_or(&Value::Null)) {
        own["target"] = Value::String(s.to_string());
    }
    if let Some(meta) = event.get("meta").filter(|m| m.as_object().is_some_and(|o| !o.is_empty())) {
        own["meta"] = meta.clone();
    }
    log["netbird"] = own;
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netbird::fixtures;
    use serde_json::json;

    #[test]
    fn a_peer_added_is_an_info_log_with_the_standard_attributes() {
        let log = log_of(&fixtures::peer_added(1042, "2026-11-06T10:00:00.123456Z"), "netbird.example.com");
        assert_eq!(log["message"], "Peer added: laptop-jordan (by Jordan Patel)");
        assert_eq!((log["status"].as_str(), log["service"].as_str(), log["ddsource"].as_str(), log["host"].as_str()), (Some("info"), Some("netbird"), Some("netbird"), Some("netbird.example.com")));
        assert_eq!(log["date"], "2026-11-06T10:00:00.123456Z");
        assert_eq!(log["evt"], json!({ "name": "peer.user.add", "outcome": "success" }));
        assert_eq!(log["usr"], json!({ "id": "google-oauth2|1001", "name": "Jordan Patel", "email": "jordan.patel@example.com" }));
        assert_eq!(log["netbird"]["event_id"], "1042");
        assert_eq!(log["netbird"]["category"], "peer");
        assert_eq!(log["netbird"]["activity"], "Peer added");
        assert_eq!(log["netbird"]["target"], "laptop-jordan");
        assert_eq!(log["netbird"]["target_id"], "cs1tnh0hhcjnqoiuebf0");
        assert_eq!(log["netbird"]["meta"]["ip"], "100.64.0.12");
    }

    #[test]
    fn overuse_is_an_error_and_revocations_warnings() {
        let overuse = log_of(&fixtures::event(7, "setupkey.overuse", "Setup key overused", "2026-11-06T10:00:00Z", json!({ "name": "ci-runners" })), "netbird.example.com");
        assert_eq!((overuse["status"].as_str(), overuse["evt"]["outcome"].as_str()), (Some("error"), Some("failure")));
        let mut blocked = fixtures::event(8, "user.block", "User blocked", "2026-11-06T10:00:00Z", json!({ "email": "sam.lee@example.com" }));
        blocked["initiator_name"] = json!("");
        let blocked = log_of(&blocked, "netbird.example.com");
        assert_eq!((blocked["status"].as_str(), blocked["message"].as_str()), (Some("warn"), Some("User blocked: sam.lee@example.com (by jordan.patel@example.com)")));
    }

    #[test]
    fn system_events_have_no_user() {
        let mut e = fixtures::event(9, "peer.login.expire", "Peer login expired", "2026-11-06T10:00:00Z", json!({}));
        for k in ["initiator_id", "initiator_name", "initiator_email"] {
            e[k] = json!("");
        }
        let log = log_of(&e, "netbird.example.com");
        assert_eq!(log["message"], "Peer login expired");
        assert!(log.get("usr").is_none() && log["netbird"].get("meta").is_none());
    }

    #[test]
    fn an_events_identity_is_stable_per_instance_and_timed_at_the_event() {
        let e = fixtures::peer_added(1042, "2026-11-06T10:00:00.123456Z");
        let (a, b) = (log_of(&e, "netbird.example.com"), log_of(&e, "netbird.example.com"));
        let id = a["_oc_ingest"]["id"].as_str().unwrap();
        assert_eq!(id, b["_oc_ingest"]["id"].as_str().unwrap(), "the same event, the same row");
        let ms = i64::from_str_radix(&id.replace('-', "")[..12], 16).unwrap();
        assert_eq!(ms, chrono::DateTime::parse_from_rfc3339("2026-11-06T10:00:00.123Z").unwrap().timestamp_millis());
        assert_ne!(log_of(&e, "vpn.other.example.com")["_oc_ingest"]["id"], a["_oc_ingest"]["id"], "ids are per management server");
        assert_ne!(log_of(&fixtures::peer_added(1043, "2026-11-06T10:00:00.123456Z"), "netbird.example.com")["_oc_ingest"]["id"], a["_oc_ingest"]["id"]);
    }
}
