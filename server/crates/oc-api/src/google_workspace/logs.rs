//! A Reports API activity as logs, one per event, as Datadog's Google Workspace integration sends them
//! (`source:gsuite`, `service:` the application: `login`, `admin`, `drive`, …): Datadog's standard attributes
//! (`evt.name`, `evt.outcome`, `usr.*`, `network.client.*`), the activity's own fields as Google answers them (`id.*`,
//! `actor.*`, `ownerDomain`), and the event under `event.*` with its parameters flattened
//! (`event.parameters.doc_title`, `event.parameters.visibility`, …). Each log's identity comes from the activity's
//! `id.uniqueQualifier` and `id.time` and the event's place in it, so reading an activity again rewrites the same rows.

use serde_json::{json, Map, Value};

pub const SOURCE: &str = "gsuite";

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A parameter's value: `value` (text), `intValue` (a number), `boolValue`, `multiValue`, `multiIntValue`,
/// `messageValue` (its parameters, as an object) or `multiMessageValue` (a list of them).
pub fn param_value(p: &Value) -> Option<Value> {
    let int = |v: &Value| v.as_str().and_then(|s| s.parse::<i64>().ok()).map(Value::from).or_else(|| v.as_i64().map(Value::from));
    if let Some(v) = p.get("value") {
        return Some(v.clone());
    }
    if let Some(v) = p.get("intValue") {
        return int(v);
    }
    if let Some(v) = p.get("boolValue") {
        return Some(v.clone());
    }
    if let Some(v) = p.get("multiValue") {
        return Some(v.clone());
    }
    if let Some(v) = p.get("multiIntValue").and_then(Value::as_array) {
        return Some(Value::Array(v.iter().filter_map(int).collect()));
    }
    if let Some(v) = p.get("messageValue") {
        return Some(params(&v["parameter"]));
    }
    if let Some(v) = p.get("multiMessageValue").and_then(Value::as_array) {
        return Some(Value::Array(v.iter().map(|m| params(&m["parameter"])).collect()));
    }
    None
}

/// A parameter list as an object (`name` → value).
pub fn params(list: &Value) -> Value {
    let mut out = Map::new();
    for p in list.as_array().into_iter().flatten() {
        if let (Some(name), Some(v)) = (text(p, "name"), param_value(p)) {
            out.insert(name.to_string(), v);
        }
    }
    Value::Object(out)
}

/// Errors: suspicious logins, government-backed attacks, hijacked or leaked accounts disabled.
pub fn is_error(name: &str) -> bool {
    let n = name.to_lowercase();
    ["suspicious", "gov_attack", "hijack", "leak", "account_disabled", "compromised"].iter().any(|w| n.contains(w))
}

/// Warnings: failed logins and challenges, actions blocked or denied, two-step verification turned off.
pub fn is_warning(name: &str) -> bool {
    let n = name.to_lowercase();
    ["failure", "failed", "blocked", "denied"].iter().any(|w| n.contains(w)) || n == "2sv_disable"
}

/// `login_failure` → `Login failure`, `ASSIGN_ROLE` → `Assign role`.
fn words(name: &str) -> String {
    let spaced = name.to_lowercase().replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// Parameters that say what an event was about, in the message.
const ABOUT: &[&str] = &["doc_title", "USER_EMAIL", "GROUP_EMAIL", "ROLE_NAME", "app_name", "application_name", "SETTING_NAME", "DEVICE_MODEL", "visibility", "login_type"];

fn message(name: &str, actor: Option<&str>, application: &str, p: &Value) -> String {
    let mut m = format!("{} ({application})", words(name));
    if let Some(a) = actor {
        m.push_str(&format!(" by {a}"));
    }
    let about: Vec<String> = ABOUT.iter().filter_map(|k| p.get(*k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(|v| format!("{k}: {v}"))).collect();
    if !about.is_empty() {
        m.push_str(&format!(" [{}]", about.join(", ")));
    }
    m
}

/// FNV-1a of the activity's identity and the event's place: the random bits of a log's identity.
fn seed(key: &str) -> u128 {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let high = fnv(0xcbf2_9ce4_8422_2325, key.as_bytes());
    ((high as u128) << 64) | fnv(high, b"google-workspace") as u128
}

/// An activity's identity: its application, `id.uniqueQualifier` and `id.time` (Google's documented key).
pub fn activity_key(activity: &Value) -> String {
    let id = &activity["id"];
    format!("{}|{}|{}", text(id, "applicationName").unwrap_or_default(), text(id, "uniqueQualifier").unwrap_or_default(), text(id, "time").unwrap_or_default())
}

/// [`activity_key`] as a number (the seen activities' key in Postgres).
pub fn activity_hash(activity: &Value) -> i64 {
    (seed(&activity_key(activity)) >> 64) as u64 as i64
}

/// The time of an activity (epoch ms).
pub fn activity_ms(activity: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text(&activity["id"], "time")?).ok().map(|t| t.timestamp_millis())
}

/// The logs of an activity: one per event.
pub fn logs_of(activity: &Value) -> Vec<Value> {
    let id = &activity["id"];
    let application = text(id, "applicationName").unwrap_or("unknown").to_string();
    let time = text(id, "time").unwrap_or_default().to_string();
    let at = chrono::DateTime::parse_from_rfc3339(&time).map(|t| t.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    let actor = &activity["actor"];
    let email = text(actor, "email");
    let key = activity_key(activity);
    let mut out = Vec::new();
    for (i, event) in activity["events"].as_array().into_iter().flatten().enumerate() {
        let name = text(event, "name").unwrap_or("unknown").to_string();
        let p = params(&event["parameters"]);
        let status = if is_error(&name) {
            "error"
        } else if is_warning(&name) {
            "warn"
        } else {
            "info"
        };
        let mut evt = json!({ "name": name });
        if is_warning(&name) && !name.contains("blocked") && name != "2sv_disable" {
            evt["outcome"] = json!("failure");
        } else if name.ends_with("_success") {
            evt["outcome"] = json!("success");
        }
        let mut log = json!({
            "date": time,
            "message": message(&name, email, &application, &p),
            "status": status,
            "service": application,
            "ddsource": SOURCE,
            "evt": evt,
            "id": id,
            "event": { "type": event["type"], "name": name, "parameters": p },
            "_oc_ingest": { "id": crate::log_normalize::uuid_v7_seeded(at, seed(&format!("{key}|{i}"))) },
        });
        if actor.is_object() {
            log["actor"] = actor.clone();
        }
        let mut usr = Map::new();
        if let Some(e) = email {
            usr.insert("email".into(), json!(e));
        }
        if let Some(p) = text(actor, "profileId") {
            usr.insert("id".into(), json!(p));
        }
        if !usr.is_empty() {
            log["usr"] = Value::Object(usr);
        }
        if let Some(ip) = text(activity, "ipAddress") {
            let mut client = json!({ "ip": ip });
            let net = &activity["networkInfo"];
            let mut geoip = Map::new();
            if let Some(c) = text(net, "regionCode") {
                geoip.insert("country".into(), json!({ "iso_code": c }));
            }
            if let Some(s) = text(net, "subdivisionCode") {
                geoip.insert("subdivision".into(), json!({ "iso_code": s }));
            }
            if !geoip.is_empty() {
                client["geoip"] = Value::Object(geoip);
            }
            if let Some(asn) = net["ipAsn"].as_array().and_then(|a| a.first()) {
                client["asn"] = json!({ "number": asn });
            }
            log["network"] = json!({ "client": client });
        }
        if let Some(d) = text(activity, "ownerDomain") {
            log["ownerDomain"] = json!(d);
        }
        out.push(log);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google_workspace::fixtures;

    #[test]
    fn a_failed_login_is_a_warning_with_the_standard_attributes() {
        let logs = logs_of(&fixtures::login_failure("-1001", "2026-11-06T10:00:00.123Z", "jordan.patel@example.com"));
        assert_eq!(logs.len(), 1);
        let log = &logs[0];
        assert_eq!((log["status"].as_str(), log["service"].as_str(), log["ddsource"].as_str(), log["date"].as_str()), (Some("warn"), Some("login"), Some("gsuite"), Some("2026-11-06T10:00:00.123Z")));
        assert_eq!(log["message"], "Login failure (login) by jordan.patel@example.com [login_type: google_password]");
        assert_eq!(log["evt"], json!({ "name": "login_failure", "outcome": "failure" }));
        assert_eq!(log["usr"], json!({ "email": "jordan.patel@example.com", "id": "104857600000000000042" }));
        assert_eq!(log["network"]["client"], json!({ "ip": "198.51.100.7", "geoip": { "country": { "iso_code": "NL" } }, "asn": { "number": 64501 } }));
        assert_eq!(log["event"]["type"], "login");
        assert_eq!(log["event"]["parameters"], json!({ "login_type": "google_password", "login_failure_type": "login_failure_invalid_password", "is_suspicious": false }));
        assert_eq!(log["id"]["uniqueQualifier"], "-1001");
        assert_eq!(log["actor"]["callerType"], "USER");
    }

    #[test]
    fn statuses_and_outcomes_by_event() {
        let one = |a: Value| logs_of(&a).remove(0);
        let ok = one(fixtures::login_success("1", "2026-11-06T10:00:00Z"));
        assert_eq!((ok["status"].as_str(), ok["evt"]["outcome"].as_str()), (Some("info"), Some("success")));
        assert_eq!(ok["event"]["parameters"]["login_challenge_method"], json!(["password", "google_authenticator"]));
        assert_eq!(one(fixtures::suspicious_login("2", "2026-11-06T10:00:00Z"))["status"], "error");
        let role = one(fixtures::role_assigned("3", "2026-11-06T10:00:00Z"));
        assert_eq!((role["status"].as_str(), role["service"].as_str(), role["evt"].get("outcome")), (Some("info"), Some("admin"), None));
        assert_eq!(role["message"], "Assign role (admin) by admin@example.com [USER_EMAIL: riley.chen@example.com, ROLE_NAME: _SEED_ADMIN_ROLE]");
        let off = one(fixtures::activity("user_accounts", "4", "2026-11-06T10:00:00Z", "sam.lee@example.com", "2sv_change", "2sv_disable", &[]));
        assert_eq!((off["status"].as_str(), off["evt"].get("outcome")), (Some("warn"), None));
        assert!(is_error("account_disabled_password_leak") && is_error("gov_attack_warning") && is_warning("risky_sensitive_action_blocked") && !is_warning("login_success"));
    }

    #[test]
    fn parameters_of_every_kind_are_flattened() {
        let list = json!([
            { "name": "doc_title", "value": "Plan" },
            { "name": "count", "intValue": "42" },
            { "name": "shared", "boolValue": true },
            { "name": "scopes", "multiValue": ["a", "b"] },
            { "name": "ids", "multiIntValue": ["1", "2"] },
            { "name": "device", "messageValue": { "parameter": [{ "name": "os", "value": "iOS" }, { "name": "managed", "boolValue": false }] } },
            { "name": "labels", "multiMessageValue": [{ "parameter": [{ "name": "id", "value": "l1" }] }] },
            { "name": "empty" }
        ]);
        assert_eq!(
            params(&list),
            json!({ "doc_title": "Plan", "count": 42, "shared": true, "scopes": ["a", "b"], "ids": [1, 2], "device": { "os": "iOS", "managed": false }, "labels": [{ "id": "l1" }] })
        );
    }

    #[test]
    fn every_event_of_an_activity_is_a_log_with_a_stable_identity_at_its_time() {
        let mut a = fixtures::shared_publicly("-77", "2026-11-06T10:00:00.250Z", "people_with_link");
        let second = json!({ "type": "access", "name": "edit", "parameters": [{ "name": "doc_title", "value": "Q3 storefront roadmap" }] });
        a["events"].as_array_mut().unwrap().push(second);
        let (logs, again) = (logs_of(&a), logs_of(&a));
        assert_eq!(logs.len(), 2);
        let ids: Vec<&str> = logs.iter().map(|l| l["_oc_ingest"]["id"].as_str().unwrap()).collect();
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids, again.iter().map(|l| l["_oc_ingest"]["id"].as_str().unwrap()).collect::<Vec<_>>(), "the same activity, the same rows");
        let ms = i64::from_str_radix(&ids[0].replace('-', "")[..12], 16).unwrap();
        assert_eq!(Some(ms), activity_ms(&a));
        assert_eq!(logs[0]["event"]["parameters"]["visibility"], "people_with_link");
        assert_eq!(logs[1]["evt"]["name"], "edit");
        // Another activity at the same time is another row.
        let other = fixtures::shared_publicly("-78", "2026-11-06T10:00:00.250Z", "people_with_link");
        assert_ne!(logs_of(&other)[0]["_oc_ingest"]["id"], logs[0]["_oc_ingest"]["id"]);
        assert_ne!(activity_hash(&other), activity_hash(&a));
    }
}
