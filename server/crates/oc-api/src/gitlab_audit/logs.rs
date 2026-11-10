//! A GitLab audit event as a log (`source:gitlab-audit-events`, `service:gitlab`): Datadog's standard attributes
//! (`evt.name`, `evt.outcome`, `usr.*`, `network.client.ip`), the event's own fields under `gitlab.*` (the entity and
//! target it is about, the author's class, the event's `details` as GitLab records them), its time as `date`, and an
//! identity from the event's entity type and id (GitLab numbers each entity type's events apart), so reading an event
//! again rewrites the same row.

use serde_json::{json, Map, Value};

pub const SOURCE: &str = "gitlab-audit-events";
pub const SERVICE: &str = "gitlab";

/// Fields of `details` that become standard or `gitlab.*` attributes (the rest stay under `gitlab.details`).
const LIFTED: &[&str] = &["author_name", "author_email", "author_class", "ip_address", "entity_path", "target_id", "target_type", "target_details", "event_name", "custom_message"];

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A number or a string as text (GitLab answers ids as either).
fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `personal_access_token_created` → `Personal access token created`.
fn words(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// The event's name: GitLab's `event_name` (16.x and later), else what its details say.
pub fn event_name(event: &Value) -> String {
    let details = &event["details"];
    if let Some(name) = text(event, "event_name").or_else(|| text(details, "event_name")) {
        return name.to_string();
    }
    if details.get("failed_login").is_some() {
        return "login_failed".into();
    }
    for (key, verb) in [("add", "added"), ("remove", "removed"), ("change", "changed")] {
        if let Some(what) = text(details, key) {
            return format!("{}_{verb}", what.to_lowercase().replace([' ', '-'], "_"));
        }
    }
    "audit_event".into()
}

/// Failures (errors): failed logins, failed creations, rejected authorizations.
pub fn is_failure(name: &str, details: &Value) -> bool {
    name.contains("failed") || name.starts_with("user_rejected") || details.get("failed_login").is_some() || text(details, "custom_message").is_some_and(|m| m.starts_with("Failed to"))
}

/// Warnings: accounts locked or blocked, administrator rights changed, impersonation.
pub fn is_warning(name: &str) -> bool {
    ["locked", "blocked", "banned", "admin_status", "impersonat", "two_factor_disabled", "disable_two_factor"].iter().any(|w| name.contains(w))
}

fn message(name: &str, author: Option<&str>, details: &Value) -> String {
    let who = author.unwrap_or("Someone");
    let on = text(details, "target_details").or_else(|| text(details, "entity_path"));
    let on = on.map(|t| format!(" on {t}")).unwrap_or_default();
    if let Some(m) = text(details, "custom_message") {
        return match author {
            Some(a) if !m.contains(a) => format!("{m} ({a})"),
            _ => m.to_string(),
        };
    }
    if let Some(change) = text(details, "change") {
        let (from, to) = (details.get("from").map(plain).unwrap_or_default(), details.get("to").map(plain).unwrap_or_default());
        return match (from.is_empty(), to.is_empty()) {
            (false, false) => format!("{who} changed {change} from {from} to {to}{on}"),
            (true, false) => format!("{who} changed {change} to {to}{on}"),
            _ => format!("{who} changed {change}{on}"),
        };
    }
    if let Some(added) = text(details, "add") {
        let role = text(details, "as").map(|r| format!(" as {r}")).unwrap_or_default();
        return format!("{who} added {added}{role}{on}");
    }
    if let Some(removed) = text(details, "remove") {
        return format!("{who} removed {removed}{on}");
    }
    format!("{} by {who}{on}", words(name))
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// FNV-1a of the event's entity type and id: the random bits of its log identity.
fn seed(key: &str) -> u128 {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let high = fnv(0xcbf2_9ce4_8422_2325, key.as_bytes());
    ((high as u128) << 64) | fnv(high, b"gitlab-audit-events") as u128
}

/// The log of a GitLab audit event, from the instance at `host`.
pub fn log_of(event: &Value, host: &str) -> Value {
    let details = event.get("details").cloned().unwrap_or(Value::Null);
    let name = event_name(event);
    let author = text(&details, "author_name");
    let created = event["created_at"].as_str().unwrap_or_default();
    let at = chrono::DateTime::parse_from_rfc3339(created).map(|t| t.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    let entity_type = text(event, "entity_type").unwrap_or("");
    let id = id_text(&event["id"]).unwrap_or_default();
    let failure = is_failure(&name, &details);
    let status = if failure {
        "error"
    } else if is_warning(&name) {
        "warn"
    } else {
        "info"
    };
    let mut log = json!({
        "date": created,
        "message": message(&name, author, &details),
        "status": status,
        "service": SERVICE,
        "ddsource": SOURCE,
        "host": host,
        "evt": { "name": name, "outcome": if failure { "failure" } else { "success" } },
        "_oc_ingest": { "id": crate::log_normalize::uuid_v7_seeded(at, seed(&format!("{entity_type}:{id}"))) },
    });
    let mut usr = Map::new();
    // Failed logins and system events have no author (-1).
    if let Some(author_id) = id_text(&event["author_id"]).filter(|a| a != "-1" && a != "0") {
        usr.insert("id".into(), Value::String(author_id));
    }
    if let Some(n) = author {
        usr.insert("name".into(), Value::String(n.to_string()));
    }
    if let Some(e) = text(&details, "author_email") {
        usr.insert("email".into(), Value::String(e.to_string()));
    }
    if !usr.is_empty() {
        log["usr"] = Value::Object(usr);
    }
    if let Some(ip) = text(&details, "ip_address") {
        log["network"] = json!({ "client": { "ip": ip } });
    }
    let mut own = Map::new();
    own.insert("audit_event_id".into(), Value::String(id));
    if !entity_type.is_empty() {
        own.insert("entity_type".into(), Value::String(entity_type.to_string()));
    }
    if let Some(e) = id_text(&event["entity_id"]).filter(|e| e != "-1") {
        own.insert("entity_id".into(), Value::String(e));
    }
    for (key, out) in [("entity_path", "entity_path"), ("target_type", "target_type"), ("target_details", "target_details"), ("author_class", "author_class"), ("custom_message", "custom_message")] {
        if let Some(v) = text(&details, key) {
            own.insert(out.into(), Value::String(v.to_string()));
        }
    }
    if let Some(t) = details.get("target_id").and_then(id_text) {
        own.insert("target_id".into(), Value::String(t));
    }
    if let Value::Object(mut rest) = details {
        rest.retain(|k, v| !LIFTED.contains(&k.as_str()) && !v.is_null());
        if !rest.is_empty() {
            own.insert("details".into(), Value::Object(rest));
        }
    }
    log["gitlab"] = Value::Object(own);
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_audit::fixtures;

    #[test]
    fn a_change_is_an_info_log_with_the_standard_attributes() {
        let log = log_of(&fixtures::changed(1001, "2026-11-06T10:00:00.123Z"), "gitlab.example.com");
        assert_eq!(log["message"], "Jordan Patel changed visibility_level from private to internal on example-org/storefront-api");
        assert_eq!(
            (log["status"].as_str(), log["service"].as_str(), log["ddsource"].as_str(), log["host"].as_str(), log["date"].as_str()),
            (Some("info"), Some("gitlab"), Some("gitlab-audit-events"), Some("gitlab.example.com"), Some("2026-11-06T10:00:00.123Z"))
        );
        assert_eq!(log["evt"], json!({ "name": "project_visibility_level_updated", "outcome": "success" }));
        assert_eq!(log["usr"], json!({ "id": "42", "name": "Jordan Patel", "email": "jordan.patel@example.com" }));
        assert_eq!(log["network"]["client"]["ip"], "203.0.113.24");
        assert_eq!(
            log["gitlab"],
            json!({
                "audit_event_id": "1001", "entity_type": "Project", "entity_id": "4211", "entity_path": "example-org/storefront-api", "target_type": "Project",
                "target_details": "example-org/storefront-api", "target_id": "4211", "author_class": "User",
                "details": { "change": "visibility_level", "from": "private", "to": "internal" }
            })
        );
    }

    #[test]
    fn failed_logins_are_errors_about_the_user_who_tried() {
        let log = log_of(&fixtures::failed_login(7, "2026-11-06T10:00:00Z", "root"), "gitlab.example.com");
        assert_eq!((log["status"].as_str(), log["evt"]["outcome"].as_str(), log["evt"]["name"].as_str()), (Some("error"), Some("failure"), Some("login_failed_with_standard_authentication")));
        assert_eq!(log["message"], "Failed to login with STANDARD authentication (root)");
        assert_eq!(log["usr"], json!({ "name": "root" }), "no author id for a user not signed in");
        assert_eq!(log["network"]["client"]["ip"], "198.51.100.7");
        assert!(log["gitlab"].get("entity_id").is_none());
        assert_eq!(log["gitlab"]["details"], json!({ "failed_login": "STANDARD" }));
    }

    #[test]
    fn older_events_without_a_name_are_named_from_their_details() {
        let mut added = fixtures::event(2, "", "2026-11-06T10:00:00Z", json!({ "add": "user_access", "as": "Developer", "target_details": "Sam Lee" }));
        added["event_name"] = Value::Null;
        added["details"].as_object_mut().unwrap().remove("event_name");
        let log = log_of(&added, "gitlab.example.com");
        assert_eq!((log["evt"]["name"].as_str(), log["message"].as_str()), (Some("user_access_added"), Some("Jordan Patel added user_access as Developer on Sam Lee")));
        let token = log_of(&fixtures::event(3, "personal_access_token_created", "2026-11-06T10:00:00Z", json!({ "custom_message": "Created personal access token with id 77" })), "gitlab.example.com");
        assert_eq!(token["message"], "Created personal access token with id 77 (Jordan Patel)");
        let blocked = log_of(&fixtures::event(4, "user_blocked", "2026-11-06T10:00:00Z", json!({})), "gitlab.example.com");
        assert_eq!((blocked["status"].as_str(), blocked["message"].as_str()), (Some("warn"), Some("User blocked by Jordan Patel on example-org/storefront-api")));
    }

    #[test]
    fn an_events_identity_is_stable_and_timed_at_the_event() {
        let e = fixtures::changed(1001, "2026-11-06T10:00:00.123Z");
        let (a, b) = (log_of(&e, "gitlab.example.com"), log_of(&e, "gitlab.example.com"));
        let id = a["_oc_ingest"]["id"].as_str().unwrap();
        assert_eq!(id, b["_oc_ingest"]["id"].as_str().unwrap(), "the same event, the same row");
        let ms = i64::from_str_radix(&id.replace('-', "")[..12], 16).unwrap();
        assert_eq!(ms, chrono::DateTime::parse_from_rfc3339("2026-11-06T10:00:00.123Z").unwrap().timestamp_millis());
        // Another entity type's event with the same id is another row.
        let mut group = e.clone();
        group["entity_type"] = json!("Group");
        assert_ne!(log_of(&group, "gitlab.example.com")["_oc_ingest"]["id"], a["_oc_ingest"]["id"]);
    }
}
