//! A Fleet activity as a log (`source:fleetdm`): Datadog's standard attributes (`evt.name`, `evt.outcome`, `usr.*`,
//! `network.client.ip`), the activity's own fields under `fleetdm.*` (its `details` as Fleet records them), its time
//! as `date`, and an identity from the instance and the activity's id, so reading an activity again rewrites the same
//! row.

use serde_json::{json, Map, Value};

/// Activities that are warnings: hosts locked or wiped, disk encryption keys read, and anything deleted.
const WARNINGS: &[&str] = &["locked_host", "wiped_host", "read_host_disk_encryption_key", "deleted_host"];

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// What the activity touched, by the details' first naming field.
fn subject(details: &Value) -> Option<&str> {
    ["policy_name", "query_name", "script_name", "software_title", "profile_name", "label_name", "pack_name", "team_name", "fleet_name", "user_name", "user_email", "name"]
        .iter()
        .find_map(|k| text(details, k))
}

/// `created_policy` → `created policy`; `user_logged_in` → `logged in` (its actor is the user).
fn verb(kind: &str) -> String {
    kind.strip_prefix("user_").filter(|rest| rest.starts_with("logged") || rest.starts_with("added_by")).unwrap_or(kind).replace('_', " ")
}

fn message(kind: &str, actor: &str, details: &Value) -> String {
    if kind == "user_failed_login" {
        return format!("Failed login for {}", text(details, "email").unwrap_or(actor));
    }
    let mut out = format!("{actor} {}", verb(kind));
    let host = text(details, "host_display_name");
    if let Some(s) = subject(details).filter(|s| Some(*s) != host) {
        out.push(' ');
        out.push_str(s);
    }
    if let Some(h) = host {
        out.push_str(if out.ends_with(&verb(kind)) && kind.ends_with("_host") { " " } else { " on " });
        out.push_str(h);
    }
    if let Some(status) = text(details, "status") {
        out.push_str(&format!(" ({status})"));
    }
    out
}

/// FNV-1a of the instance and the activity's id: the random bits of its log identity.
fn seed(instance: &str, id: &str) -> u128 {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let high = fnv(fnv(0xcbf2_9ce4_8422_2325, instance.as_bytes()), id.as_bytes());
    ((high as u128) << 64) | fnv(high, b"fleetdm") as u128
}

/// The log of a Fleet activity, from the Fleet server at `host`.
pub fn log_of(activity: &Value, host: &str) -> Value {
    let kind = text(activity, "type").unwrap_or("activity");
    let details = activity.get("details").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    let fleet_initiated = activity["fleet_initiated"].as_bool().unwrap_or(false);
    let actor_name = text(activity, "actor_full_name").or_else(|| text(activity, "actor_email"));
    let actor = actor_name.unwrap_or(if fleet_initiated { "Fleet" } else { "Someone" });
    let created = activity["created_at"].as_str().unwrap_or_default();
    let at = chrono::DateTime::parse_from_rfc3339(created).map(|t| t.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    let id = match &activity["id"] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    let failed = kind == "user_failed_login" || text(&details, "status").is_some_and(|s| s.contains("fail"));
    let status = if failed {
        "error"
    } else if WARNINGS.contains(&kind) || kind.starts_with("deleted_") {
        "warn"
    } else {
        "info"
    };
    let mut log = json!({
        "date": created,
        "message": message(kind, actor, &details),
        "status": status,
        "service": "fleetdm",
        "ddsource": "fleetdm",
        "host": host,
        "evt": { "name": kind, "outcome": if failed { "failure" } else { "success" } },
        "_oc_ingest": { "id": crate::log_normalize::uuid_v7_seeded(at, seed(host, &id)) },
    });
    let mut usr = Map::new();
    if let Some(actor_id) = activity.get("actor_id").filter(|v| !v.is_null()) {
        usr.insert("id".into(), Value::String(actor_id.as_str().map_or_else(|| actor_id.to_string(), str::to_string)));
    }
    if let Some(name) = text(activity, "actor_full_name") {
        usr.insert("name".into(), Value::String(name.to_string()));
    }
    if let Some(email) = text(activity, "actor_email").or_else(|| (kind == "user_failed_login").then(|| text(&details, "email")).flatten()) {
        usr.insert("email".into(), Value::String(email.to_string()));
    }
    if !usr.is_empty() {
        log["usr"] = Value::Object(usr);
    }
    if let Some(ip) = text(&details, "public_ip") {
        log["network"] = json!({ "client": { "ip": ip } });
    }
    let mut own = json!({ "activity_id": id, "fleet_initiated": fleet_initiated, "details": details });
    if let Some(h) = text(&details, "host_display_name") {
        own["host_display_name"] = Value::String(h.to_string());
    }
    if let Some(h) = details.get("host_id").filter(|v| !v.is_null()) {
        own["host_id"] = h.clone();
    }
    if let Some(team) = text(&details, "team_name").or_else(|| text(&details, "fleet_name")) {
        own["team"] = Value::String(team.to_string());
    }
    log["fleetdm"] = own;
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleetdm::fixtures;
    use serde_json::json;

    const HOST: &str = "fleet.example.com";

    #[test]
    fn a_policy_created_is_an_info_log_with_the_standard_attributes() {
        let log = log_of(&fixtures::created_policy(25, "2026-11-06T10:00:00Z"), HOST);
        assert_eq!(log["message"], "Jordan Patel created policy Disk encryption enabled");
        assert_eq!((log["status"].as_str(), log["service"].as_str(), log["ddsource"].as_str(), log["host"].as_str()), (Some("info"), Some("fleetdm"), Some("fleetdm"), Some(HOST)));
        assert_eq!(log["date"], "2026-11-06T10:00:00Z");
        assert_eq!(log["evt"], json!({ "name": "created_policy", "outcome": "success" }));
        assert_eq!(log["usr"], json!({ "id": "3", "name": "Jordan Patel", "email": "jordan.patel@example.com" }));
        assert_eq!(log["fleetdm"]["activity_id"], "25");
        assert_eq!(log["fleetdm"]["details"]["policy_id"], 12);
        assert!(log.get("network").is_none());
    }

    #[test]
    fn software_and_scripts_name_their_host_and_failures_are_errors() {
        let installed = log_of(
            &fixtures::activity(26, "installed_software", "2026-11-06T10:00:00Z", json!({ "status": "failed_install", "host_id": 1, "host_display_name": "Jordan's MacBook Pro", "software_title": "zoom.us.app" })),
            HOST,
        );
        assert_eq!(installed["message"], "Jordan Patel installed software zoom.us.app on Jordan's MacBook Pro (failed_install)");
        assert_eq!((installed["status"].as_str(), installed["evt"]["outcome"].as_str()), (Some("error"), Some("failure")));
        assert_eq!((installed["fleetdm"]["host_id"].as_i64(), installed["fleetdm"]["host_display_name"].as_str()), (Some(1), Some("Jordan's MacBook Pro")));
        let script = log_of(&fixtures::activity(27, "ran_script", "2026-11-06T10:00:00Z", json!({ "host_id": 2, "host_display_name": "i-0a1b2c3d4e5f60001", "script_name": "rotate-logs.sh" })), HOST);
        assert_eq!((script["status"].as_str(), script["message"].as_str()), (Some("info"), Some("Jordan Patel ran script rotate-logs.sh on i-0a1b2c3d4e5f60001")));
    }

    #[test]
    fn logins_hosts_wiped_and_deletions() {
        let mut failed = fixtures::activity(28, "user_failed_login", "2026-11-06T10:00:00Z", json!({ "email": "sam.lee@example.com", "public_ip": "203.0.113.24" }));
        for k in ["actor_full_name", "actor_email", "actor_id"] {
            failed[k] = Value::Null;
        }
        let log = log_of(&failed, HOST);
        assert_eq!((log["status"].as_str(), log["message"].as_str()), (Some("error"), Some("Failed login for sam.lee@example.com")));
        assert_eq!(log["usr"], json!({ "email": "sam.lee@example.com" }));
        assert_eq!(log["network"]["client"]["ip"], "203.0.113.24");
        let login = log_of(&fixtures::activity(29, "user_logged_in", "2026-11-06T10:00:00Z", json!({ "public_ip": "203.0.113.24" })), HOST);
        assert_eq!((login["status"].as_str(), login["message"].as_str()), (Some("info"), Some("Jordan Patel logged in")));
        let wiped = log_of(&fixtures::activity(30, "wiped_host", "2026-11-06T10:00:00Z", json!({ "host_id": 3, "host_display_name": "desktop-sam" })), HOST);
        assert_eq!((wiped["status"].as_str(), wiped["message"].as_str()), (Some("warn"), Some("Jordan Patel wiped host desktop-sam")));
        let deleted = log_of(&fixtures::activity(31, "deleted_saved_query", "2026-11-06T10:00:00Z", json!({ "query_name": "USB devices" })), HOST);
        assert_eq!((deleted["status"].as_str(), deleted["message"].as_str()), (Some("warn"), Some("Jordan Patel deleted saved query USB devices")));
        let mut automatic = fixtures::activity(32, "transferred_hosts", "2026-11-06T10:00:00Z", json!({ "team_name": "Workstations", "host_ids": [1] }));
        automatic["fleet_initiated"] = json!(true);
        automatic["actor_full_name"] = Value::Null;
        automatic["actor_email"] = Value::Null;
        let log = log_of(&automatic, HOST);
        assert_eq!(log["message"], "Fleet transferred hosts Workstations");
        assert_eq!((log["fleetdm"]["fleet_initiated"].as_bool(), log["fleetdm"]["team"].as_str()), (Some(true), Some("Workstations")));
    }

    #[test]
    fn an_activitys_identity_is_stable_per_instance_and_timed_at_the_activity() {
        let a = fixtures::created_policy(25, "2026-11-06T10:00:00.123Z");
        let id = log_of(&a, HOST)["_oc_ingest"]["id"].as_str().unwrap().to_string();
        assert_eq!(id, log_of(&a, HOST)["_oc_ingest"]["id"].as_str().unwrap(), "the same activity, the same row");
        assert_eq!(&id[14..15], "7", "a UUIDv7");
        let ms = i64::from_str_radix(&id.replace('-', "")[..12], 16).unwrap();
        assert_eq!(ms, chrono::DateTime::parse_from_rfc3339("2026-11-06T10:00:00.123Z").unwrap().timestamp_millis());
        assert_ne!(log_of(&fixtures::created_policy(26, "2026-11-06T10:00:00.123Z"), HOST)["_oc_ingest"]["id"], id);
        assert_ne!(log_of(&a, "fleet.other.example.com")["_oc_ingest"]["id"], id, "another instance's activity 25");
    }
}
