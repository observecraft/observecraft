//! Monitors, Cloud SIEM detection rules and log indexes as Vanta custom resources (`custom_resource`, one Developer
//! Console resource each, base type CustomResource), as Vanta reads Datadog's infrastructure monitors and Cloud SIEM's
//! rules and log indexes for its monitoring and SIEM tests. Their properties are under `customProperties`, with the
//! schema of [`schema`] (JSON Type Definition), which the resource is created with in Vanta.

use serde_json::{json, Value};

use crate::error::ApiError;
use crate::routes::downtimes::{load_downtimes, Downtime};
use crate::routes::monitor_engine::recipients;
use crate::state::AppState;

/// The custom resources' schemas (JSON Type Definition), by type.
pub fn schema(kind: &str) -> Option<Value> {
    let props: &[(&str, &str)] = match kind {
        "monitor" => &[("monitorType", "string"), ("query", "string"), ("priority", "string"), ("status", "string"), ("notifies", "boolean"), ("recipients", "string"), ("muted", "boolean"), ("tags", "string")],
        "detection_rule" => &[("enabled", "boolean"), ("severity", "string"), ("query", "string"), ("notifies", "boolean"), ("isDefault", "boolean"), ("contentPack", "string"), ("tags", "string")],
        "log_index" => &[("filter", "string"), ("retentionDays", "int32"), ("flexRetentionDays", "int32"), ("dailyLimit", "float64"), ("exclusionFilters", "int32"), ("position", "int32")],
        _ => return None,
    };
    Some(json!({ "properties": props.iter().map(|(name, t)| (name.to_string(), json!({ "type": t }))).collect::<serde_json::Map<_, _>>() }))
}

/// A monitor as read for Vanta.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub query: String,
    pub message: String,
    pub priority: Option<i16>,
    pub status: String,
    pub tags: Vec<String>,
}

/// A monitor as a custom resource (`muted`: a downtime silences all of it now).
pub fn monitor_resource(m: &Monitor, muted: bool, ui: &str) -> Value {
    let to = recipients(&m.message);
    json!({
        "uniqueId": format!("monitor:{}", m.id),
        "displayName": m.name,
        "externalUrl": format!("{ui}/monitors/{}", m.id),
        "customProperties": {
            "monitorType": m.kind,
            "query": m.query,
            "priority": m.priority.map_or_else(|| "none".to_string(), |p| format!("P{p}")),
            "status": m.status,
            "notifies": !to.is_empty(),
            "recipients": to.join(", "),
            "muted": muted,
            "tags": m.tags.join(", "),
        },
    })
}

/// Whether a downtime without a scope silences the whole monitor at `now`.
pub fn muted(downtimes: &[Downtime], id: i64, tags: &[String], now: i64) -> bool {
    downtimes.iter().any(|d| d.scope.is_empty() && d.applies_to(id, tags) && d.window().active_at(now).is_some())
}

/// The org's monitors (drafts aside).
pub async fn monitors(st: &AppState, org: &str, now: i64) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<Monitor> = sqlx::query_as("SELECT id, name, type AS kind, query, message, priority, status, tags FROM monitors WHERE org_id = $1 AND NOT draft ORDER BY id").bind(org).fetch_all(&st.postgres).await?;
    let downtimes = load_downtimes(st, org).await?;
    Ok(rows.iter().map(|m| monitor_resource(m, muted(&downtimes, m.id, &m.tags, now), &st.config.public_url)).collect())
}

/// A Cloud SIEM detection rule as read for Vanta.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub is_default: bool,
    pub pack: Option<String>,
    pub queries: sqlx::types::Json<Value>,
    pub cases: sqlx::types::Json<Value>,
    pub tags: Vec<String>,
}

/// A detection rule as a custom resource.
pub fn rule_resource(r: &Rule, ui: &str) -> Value {
    let rank = |s: &str| ["info", "low", "medium", "high", "critical"].iter().position(|x| *x == s).unwrap_or(0);
    let cases = r.cases.0.as_array().cloned().unwrap_or_default();
    let severity = cases.iter().filter_map(|c| c["status"].as_str()).max_by_key(|s| rank(s)).unwrap_or("info").to_string();
    let notifies = cases.iter().any(|c| c["notifications"].as_array().is_some_and(|n| !n.is_empty()));
    json!({
        "uniqueId": format!("detection_rule:{}", r.id),
        "displayName": r.name,
        "externalUrl": format!("{ui}/security/siem/rules/{}", r.id),
        "customProperties": {
            "enabled": r.enabled,
            "severity": severity,
            "query": r.queries.0[0]["query"].as_str().unwrap_or_default(),
            "notifies": notifies,
            "isDefault": r.is_default,
            "contentPack": r.pack.clone().unwrap_or_default(),
            "tags": r.tags.join(", "),
        },
    })
}

/// The org's Cloud SIEM detection rules (`log_detection`).
pub async fn detection_rules(st: &AppState, org: &str) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<Rule> = sqlx::query_as("SELECT id, name, is_enabled AS enabled, is_default, pack, queries, cases, tags FROM security_rules WHERE org_id = $1 AND type = 'log_detection' ORDER BY name, id")
        .bind(org)
        .fetch_all(&st.postgres)
        .await?;
    Ok(rows.iter().map(|r| rule_resource(r, &st.config.public_url)).collect())
}

/// A log index as read for Vanta.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct LogIndex {
    pub id: String,
    pub name: String,
    pub filter: String,
    pub retention_days: i32,
    pub flex_retention_days: Option<i32>,
    pub daily_quota: Option<i64>,
    pub exclusion_filters: sqlx::types::Json<Value>,
    pub position: i32,
}

/// A log index as a custom resource.
pub fn log_index_resource(i: &LogIndex, ui: &str) -> Value {
    let exclusions = i.exclusion_filters.0.as_array().map_or(0, |f| f.iter().filter(|x| x["enabled"].as_bool().unwrap_or(true)).count());
    json!({
        "uniqueId": format!("log_index:{}", i.id),
        "displayName": i.name,
        "externalUrl": format!("{ui}/logs/indexes"),
        "customProperties": {
            "filter": i.filter,
            "retentionDays": i.retention_days,
            "flexRetentionDays": i.flex_retention_days.unwrap_or(0),
            "dailyLimit": i.daily_quota.unwrap_or(0) as f64,
            "exclusionFilters": exclusions,
            "position": i.position,
        },
    })
}

/// The org's log indexes, in routing order.
pub async fn log_indexes(st: &AppState, org: &str) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<LogIndex> = sqlx::query_as("SELECT id::text, name, filter, retention_days, flex_retention_days, daily_quota, exclusion_filters, position FROM log_indexes WHERE org_id = $1 ORDER BY position, name")
        .bind(org)
        .fetch_all(&st.postgres)
        .await?;
    Ok(rows.iter().map(|i| log_index_resource(i, &st.config.public_url)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: &str = "https://observecraft.example.com";

    fn monitor() -> Monitor {
        Monitor {
            id: 42,
            name: "High error rate on checkout-api".into(),
            kind: "metric".into(),
            query: "avg(last_5m):sum:trace.http.request.errors{service:checkout-api} > 10".into(),
            message: "Errors are up. @slack-alerts @oncall@example.com".into(),
            priority: Some(2),
            status: "ok".into(),
            tags: vec!["team:checkout".into(), "env:prod".into()],
        }
    }

    #[test]
    fn every_custom_type_has_a_schema_of_its_properties() {
        for (kind, sample) in [
            ("monitor", monitor_resource(&monitor(), false, UI)),
            ("detection_rule", rule_resource(&rule(), UI)),
            ("log_index", log_index_resource(&index(), UI)),
        ] {
            let schema = schema(kind).unwrap_or_else(|| panic!("{kind}"));
            let props = schema["properties"].as_object().unwrap();
            let sent = sample["customProperties"].as_object().unwrap();
            assert_eq!(props.keys().collect::<Vec<_>>(), sent.keys().collect::<Vec<_>>(), "{kind}: the schema has exactly the properties sent");
            for (name, value) in sent {
                let want = props[name]["type"].as_str().unwrap();
                let ok = match want {
                    "string" => value.is_string(),
                    "boolean" => value.is_boolean(),
                    "int32" => value.is_i64(),
                    "float64" => value.is_number(),
                    _ => false,
                };
                assert!(ok, "{kind}.{name}: {value} isn't a {want}");
            }
        }
        assert!(schema("user_account").is_none());
    }

    #[test]
    fn a_monitor_says_who_it_notifies_and_whether_it_is_muted() {
        assert_eq!(
            monitor_resource(&monitor(), false, UI),
            json!({
                "uniqueId": "monitor:42",
                "displayName": "High error rate on checkout-api",
                "externalUrl": "https://observecraft.example.com/monitors/42",
                "customProperties": {
                    "monitorType": "metric",
                    "query": "avg(last_5m):sum:trace.http.request.errors{service:checkout-api} > 10",
                    "priority": "P2",
                    "status": "ok",
                    "notifies": true,
                    "recipients": "slack-alerts, oncall@example.com",
                    "muted": false,
                    "tags": "team:checkout, env:prod",
                },
            })
        );
        let mut m = monitor();
        m.message = "No handles here".into();
        m.priority = None;
        let v = monitor_resource(&m, true, UI);
        assert_eq!(v["customProperties"]["notifies"], false);
        assert_eq!(v["customProperties"]["recipients"], "");
        assert_eq!(v["customProperties"]["priority"], "none");
        assert_eq!(v["customProperties"]["muted"], true);
    }

    fn downtime(monitor_id: Option<i64>, tags: &[&str], scope: &[&str], start: i64, end: Option<i64>) -> Downtime {
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        Downtime {
            id: 1,
            monitor_id,
            monitor_name: None,
            monitor_tags: strings(tags),
            scope: strings(scope),
            message: String::new(),
            start_at: start,
            end_at: end,
            recurrence: None,
            recur_until: None,
            creator_id: None,
            creator_name: None,
            created_at: start,
            canceled_at: None,
            weekdays: vec![],
        }
    }

    #[test]
    fn only_an_active_unscoped_downtime_of_the_monitor_mutes_it() {
        let tags = vec!["team:checkout".to_string(), "env:prod".to_string()];
        let now = 1_795_000_000_000;
        assert!(muted(&[downtime(Some(42), &[], &[], now - 1000, None)], 42, &tags, now));
        assert!(muted(&[downtime(None, &["team:checkout"], &[], now - 1000, Some(now + 1000))], 42, &tags, now), "by its tags");
        assert!(!muted(&[downtime(Some(7), &[], &[], now - 1000, None)], 42, &tags, now), "another monitor");
        assert!(!muted(&[downtime(Some(42), &[], &[], now - 2000, Some(now - 1000))], 42, &tags, now), "ended");
        assert!(!muted(&[downtime(Some(42), &[], &["host:i-0a1b2c3d4e5f60001"], now - 1000, None)], 42, &tags, now), "only some groups");
    }

    fn rule() -> Rule {
        Rule {
            id: "abc-def-ghi".into(),
            name: "Brute force on console logins".into(),
            enabled: true,
            is_default: true,
            pack: Some("aws-cloudtrail".into()),
            queries: sqlx::types::Json(json!([{ "name": "a", "query": "source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure", "aggregation": "count" }])),
            cases: sqlx::types::Json(json!([
                { "name": "", "status": "medium", "condition": "a > 5", "notifications": [] },
                { "name": "", "status": "high", "condition": "a > 25", "notifications": ["@slack-security"] },
            ])),
            tags: vec!["source:cloudtrail".into(), "tactic:TA0006-credential-access".into()],
        }
    }

    #[test]
    fn a_detection_rule_has_its_highest_severity_and_notifications() {
        assert_eq!(
            rule_resource(&rule(), UI),
            json!({
                "uniqueId": "detection_rule:abc-def-ghi",
                "displayName": "Brute force on console logins",
                "externalUrl": "https://observecraft.example.com/security/siem/rules/abc-def-ghi",
                "customProperties": {
                    "enabled": true,
                    "severity": "high",
                    "query": "source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure",
                    "notifies": true,
                    "isDefault": true,
                    "contentPack": "aws-cloudtrail",
                    "tags": "source:cloudtrail, tactic:TA0006-credential-access",
                },
            })
        );
        let mut r = rule();
        r.cases = sqlx::types::Json(json!([{ "status": "info", "notifications": [] }]));
        r.pack = None;
        let v = rule_resource(&r, UI);
        assert_eq!((v["customProperties"]["severity"].as_str(), v["customProperties"]["notifies"].as_bool(), v["customProperties"]["contentPack"].as_str()), (Some("info"), Some(false), Some("")));
    }

    fn index() -> LogIndex {
        LogIndex {
            id: "00000000-0000-4000-8000-0000000000e1".into(),
            name: "main".into(),
            filter: "".into(),
            retention_days: 15,
            flex_retention_days: Some(180),
            daily_quota: None,
            exclusion_filters: sqlx::types::Json(json!([{ "name": "health checks", "query": "@http.url_details.path:/health", "sampleRate": 100, "enabled": true }, { "name": "debug", "query": "status:debug", "enabled": false }])),
            position: 0,
        }
    }

    #[test]
    fn a_log_index_has_its_retention_and_limits() {
        assert_eq!(
            log_index_resource(&index(), UI),
            json!({
                "uniqueId": "log_index:00000000-0000-4000-8000-0000000000e1",
                "displayName": "main",
                "externalUrl": "https://observecraft.example.com/logs/indexes",
                "customProperties": {
                    "filter": "",
                    "retentionDays": 15,
                    "flexRetentionDays": 180,
                    "dailyLimit": 0.0,
                    "exclusionFilters": 1,
                    "position": 0,
                },
            })
        );
        let mut i = index();
        i.flex_retention_days = None;
        i.daily_quota = Some(5_000_000_000);
        let v = log_index_resource(&i, UI);
        assert_eq!((v["customProperties"]["flexRetentionDays"].as_i64(), v["customProperties"]["dailyLimit"].as_f64()), (Some(0), Some(5e9)));
    }

    #[tokio::test]
    async fn the_orgs_monitors_rules_and_indexes_are_read() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "vanta-custom").await, crate::test_support::new_org(st, "vanta-custom").await);
        for (o, name, draft) in [(&org, "Checkout errors", false), (&org, "Draft monitor", true), (&other, "Other org", false)] {
            sqlx::query("INSERT INTO monitors (org_id, name, query, definition, message, draft, tags) VALUES ($1, $2, 'avg(last_5m):avg:system.cpu.user{*} > 90', '{}', '@slack-alerts', $3, '{team:checkout}')")
                .bind(o)
                .bind(name)
                .bind(draft)
                .execute(&st.postgres)
                .await
                .unwrap();
        }
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM monitors WHERE org_id = $1 AND name = 'Checkout errors'").bind(&org).fetch_one(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO downtimes (org_id, monitor_id, monitor_tags, scope, message, start_at) VALUES ($1, $2, '{}', '{}', '', now() - interval '1 hour')").bind(&org).bind(id).execute(&st.postgres).await.unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let sent = monitors(st, &org, now).await.unwrap();
        assert_eq!(sent.iter().map(|m| (m["displayName"].as_str().unwrap(), m["customProperties"]["muted"].as_bool().unwrap())).collect::<Vec<_>>(), [("Checkout errors", true)]);

        for (o, rid, kind) in [(&org, "siem-1", "log_detection"), (&org, "wp-1", "workload_security"), (&other, "siem-2", "log_detection")] {
            sqlx::query("INSERT INTO security_rules (org_id, id, name, type) VALUES ($1, $2, $2, $3)").bind(o).bind(rid).bind(kind).execute(&st.postgres).await.unwrap();
        }
        let rules = detection_rules(st, &org).await.unwrap();
        assert_eq!(rules.iter().map(|r| r["displayName"].as_str().unwrap()).collect::<Vec<_>>(), ["siem-1"], "Cloud SIEM's log detection rules only");

        for (o, name, position) in [(&org, "audit", 1), (&org, "main", 0), (&other, "theirs", 0)] {
            sqlx::query("INSERT INTO log_indexes (org_id, name, retention_days, position) VALUES ($1, $2, 30, $3)").bind(o).bind(name).bind(position).execute(&st.postgres).await.unwrap();
        }
        let indexes = log_indexes(st, &org).await.unwrap();
        assert_eq!(indexes.iter().map(|i| i["displayName"].as_str().unwrap()).collect::<Vec<_>>(), ["main", "audit"]);
    }
}
