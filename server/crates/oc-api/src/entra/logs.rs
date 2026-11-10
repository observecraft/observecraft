//! Graph's events as logs shaped as Azure's diagnostic settings export Entra ID's logs (the record Datadog's Azure
//! forwarder sends from an Event Hub, `source:azure.activedirectory`): `category` (`SignInLogs`, `AuditLogs`,
//! `UserRiskEvents`), `operationName`, `resultType`, `callerIpAddress`, `identity`, `tenantId`, `time`, …, and the
//! Graph object itself under `properties` (diagnostic settings export the same camelCase fields), so the
//! integration's log pipeline (integrations-core `azure_active_directory`), its facets and Cloud SIEM's rules read
//! them as they read the Event Hub's. Null fields are left out, as the export does. A log's identity comes from the
//! event's id, so an event read again rewrites the same row.

use serde_json::{json, Map, Value};

use super::api::Stream;

pub const SOURCE: &str = "azure.activedirectory";

fn text<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(*k)).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The value without its null fields (recursively), as diagnostic settings export it.
fn without_nulls(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), without_nulls(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}

/// FNV-1a of the event's id: the random bits of its log identity.
fn seed(id: &str, stream: Stream) -> u128 {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let high = fnv(0xcbf2_9ce4_8422_2325, id.as_bytes());
    ((high as u128) << 64) | fnv(high, stream.scope().as_bytes()) as u128
}

/// The event's time (its stream's time field), epoch ms.
pub fn event_ms(stream: Stream, event: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(event[stream.time_field()].as_str()?).ok().map(|t| t.timestamp_millis())
}

/// The log of a Graph event of `stream` from tenant `tenant`.
pub fn log_of(stream: Stream, event: &Value, tenant: &str) -> Value {
    let properties = without_nulls(event);
    let time = event[stream.time_field()].as_str().unwrap_or_default();
    let at = chrono::DateTime::parse_from_rfc3339(time).map(|t| t.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    let id = event["id"].as_str().unwrap_or_default();
    let mut log = Map::new();
    let mut put = |k: &str, v: Value| {
        log.insert(k.to_string(), v);
    };
    put("time", json!(time));
    put("resourceId", json!(format!("/tenants/{tenant}/providers/Microsoft.aadiam")));
    put("operationVersion", json!("1.0"));
    put("tenantId", json!(tenant));
    put("resultSignature", json!("None"));
    put("durationMs", json!(0));
    put("Level", json!(4));
    if let Some(c) = text(event, &["correlationId"]) {
        put("correlationId", json!(c));
    }
    let message = match stream {
        Stream::SignIns => {
            let user = text(event, &["userPrincipalName"]).or(text(event, &["userDisplayName"])).unwrap_or("someone");
            let app = text(event, &["appDisplayName"]).unwrap_or("an application");
            let code = event["status"]["errorCode"].as_i64().unwrap_or(0);
            put("category", json!("SignInLogs"));
            put("operationName", json!("Sign-in activity"));
            put("resultType", json!(code.to_string()));
            if let Some(ip) = text(event, &["ipAddress"]) {
                put("callerIpAddress", json!(ip));
            }
            if let Some(name) = text(event, &["userDisplayName"]) {
                put("identity", json!(name));
            }
            if let Some(country) = text(event, &["location", "countryOrRegion"]) {
                put("location", json!(country));
            }
            if code == 0 {
                format!("{user} signed in to {app}")
            } else {
                let reason = text(event, &["status", "failureReason"]).unwrap_or("failed");
                put("resultDescription", json!(reason));
                format!("Sign-in of {user} to {app} failed: {reason}")
            }
        }
        Stream::Audits => {
            let activity = text(event, &["activityDisplayName"]).unwrap_or("Audit event");
            put("category", json!("AuditLogs"));
            put("operationName", json!(activity));
            if let Some(ip) = text(event, &["initiatedBy", "user", "ipAddress"]) {
                put("callerIpAddress", json!(ip));
            }
            let by = text(event, &["initiatedBy", "user", "userPrincipalName"]).or(text(event, &["initiatedBy", "app", "displayName"]));
            if let Some(by) = by {
                put("identity", json!(by));
            }
            let target = event["targetResources"].as_array().and_then(|t| t.iter().find_map(|r| text(r, &["userPrincipalName"]).or(text(r, &["displayName"]))));
            match (by, target) {
                (Some(by), Some(target)) => format!("{activity}: {target} (by {by})"),
                (Some(by), None) => format!("{activity} (by {by})"),
                (None, Some(target)) => format!("{activity}: {target}"),
                (None, None) => activity.to_string(),
            }
        }
        Stream::Risk => {
            let kind = text(event, &["riskEventType"]).unwrap_or("risk");
            let user = text(event, &["userPrincipalName"]).unwrap_or("a user");
            put("category", json!("UserRiskEvents"));
            put("operationName", json!("User Risk Detection"));
            if let Some(ip) = text(event, &["ipAddress"]) {
                put("callerIpAddress", json!(ip));
            }
            if let Some(name) = text(event, &["userDisplayName"]) {
                put("identity", json!(name));
            }
            format!("Risk detected for {user}: {kind} ({} risk)", text(event, &["riskLevel"]).unwrap_or("unknown"))
        }
    };
    put("properties", properties);
    put("date", json!(time));
    put("message", json!(message));
    put("ddsource", json!(SOURCE));
    put("service", json!("azure"));
    put("_oc_ingest", json!({ "id": crate::log_normalize::uuid_v7_seeded(at, seed(id, stream)) }));
    Value::Object(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entra::fixtures::{self, TENANT};

    #[test]
    fn a_sign_in_is_a_sign_in_logs_record() {
        let log = log_of(Stream::SignIns, &fixtures::sign_in("b95c8eca-487b-4631-b6e5-0469cb7c0800", "2026-11-06T10:00:00.1234567Z", "jordan.patel@example.com", 0), TENANT);
        assert_eq!((log["category"].as_str(), log["operationName"].as_str(), log["resultType"].as_str()), (Some("SignInLogs"), Some("Sign-in activity"), Some("0")));
        assert_eq!((log["ddsource"].as_str(), log["service"].as_str()), (Some("azure.activedirectory"), Some("azure")));
        assert_eq!(log["resourceId"], format!("/tenants/{TENANT}/providers/Microsoft.aadiam"));
        assert_eq!((log["tenantId"].as_str(), log["time"].as_str(), log["date"].as_str()), (Some(TENANT), Some("2026-11-06T10:00:00.1234567Z"), Some("2026-11-06T10:00:00.1234567Z")));
        assert_eq!((log["callerIpAddress"].as_str(), log["identity"].as_str(), log["location"].as_str()), (Some("203.0.113.24"), Some("Jordan Patel"), Some("RO")));
        assert_eq!(log["properties"]["userPrincipalName"], "jordan.patel@example.com");
        assert_eq!(log["properties"]["status"], json!({ "errorCode": 0, "failureReason": "Other." }), "nulls left out");
        assert!(log["properties"]["deviceDetail"].get("displayName").is_none());
        assert!(log.get("resultDescription").is_none(), "a success has no failure reason");
        assert_eq!(log["message"], "jordan.patel@example.com signed in to Azure Portal");
        let failed = log_of(Stream::SignIns, &fixtures::sign_in("2", "2026-11-06T10:00:00Z", "jordan.patel@example.com", 50126), TENANT);
        assert_eq!((failed["resultType"].as_str(), failed["resultDescription"].as_str()), (Some("50126"), Some("Invalid username or password or Invalid on-premise username or password.")));
        assert!(failed["message"].as_str().unwrap().starts_with("Sign-in of jordan.patel@example.com to Azure Portal failed"));
    }

    #[test]
    fn an_audit_is_an_audit_logs_record_and_a_risk_detection_a_user_risk_event() {
        let log = log_of(Stream::Audits, &fixtures::role_assigned("Directory_5d3c_1", "2026-11-06T10:00:00Z", "Global Administrator"), TENANT);
        assert_eq!((log["category"].as_str(), log["operationName"].as_str(), log["properties"]["category"].as_str()), (Some("AuditLogs"), Some("Add member to role"), Some("RoleManagement")));
        assert_eq!((log["callerIpAddress"].as_str(), log["identity"].as_str()), (Some("198.51.100.7"), Some("sam.lee@example.com")));
        assert_eq!(log["message"], "Add member to role: riley.chen@example.com (by sam.lee@example.com)");
        assert!(log["properties"].get("initiatedBy").unwrap().get("app").is_none());
        let risk = log_of(Stream::Risk, &fixtures::risk("r-1", "2026-11-06T10:00:00Z", "high"), TENANT);
        assert_eq!((risk["category"].as_str(), risk["properties"]["riskLevel"].as_str(), risk["time"].as_str()), (Some("UserRiskEvents"), Some("high"), Some("2026-11-06T10:00:00Z")));
        assert_eq!(risk["message"], "Risk detected for jordan.patel@example.com: unfamiliarFeatures (high risk)");
    }

    #[test]
    fn an_events_identity_is_stable_per_stream_and_timed_at_the_event() {
        let e = fixtures::sign_in("b95c8eca", "2026-11-06T10:00:00.123Z", "jordan.patel@example.com", 0);
        let id = |s: Stream, e: &Value| log_of(s, e, TENANT)["_oc_ingest"]["id"].as_str().unwrap().to_string();
        assert_eq!(id(Stream::SignIns, &e), id(Stream::SignIns, &e));
        let ms = i64::from_str_radix(&id(Stream::SignIns, &e).replace('-', "")[..12], 16).unwrap();
        assert_eq!(ms, chrono::DateTime::parse_from_rfc3339("2026-11-06T10:00:00.123Z").unwrap().timestamp_millis());
        assert_eq!(event_ms(Stream::SignIns, &e), Some(ms));
        let other = fixtures::sign_in("c95c8eca", "2026-11-06T10:00:00.123Z", "jordan.patel@example.com", 0);
        assert_ne!(id(Stream::SignIns, &e), id(Stream::SignIns, &other));
    }
}
