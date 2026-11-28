//! Security alerts as Vanta `security_task` resources, for its security alert SLA tests, with their priority from the
//! severity:
//! - Cloud Security's misconfigurations (the account scans' failed checks, `posture_findings`), as Vanta reads Cloud
//!   Security's findings from Datadog: open while the check fails, closed when it passes again or was muted. Checks that
//!   never failed aren't findings.
//! - Security signals (`security_signals`: Cloud SIEM, App & API Protection, Workload Protection): open or under review,
//!   closed once archived. Suppressed signals aren't sent.
//!
//! Resolved ones are kept [`RESOLVED_DAYS`] days, so Vanta sees when they were fixed.

use serde_json::{json, Value};

use crate::state::AppState;

/// How long a resolved finding is still sent.
pub const RESOLVED_DAYS: i32 = 90;

/// A finding as read for Vanta (times in epoch ms).
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Finding {
    pub id: String,
    pub provider: String,
    pub account_id: String,
    pub check_id: String,
    /// The check's title (its id when the catalog has none).
    pub title: String,
    pub resource_uid: String,
    pub resource_name: String,
    pub resource_type: String,
    pub region: String,
    pub severity: String,
    /// fail | pass
    pub evaluation: String,
    pub muted: bool,
    pub first_seen: i64,
    pub last_seen: i64,
    pub changed_at: i64,
    pub muted_at: Option<i64>,
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The tasks' creator, which Vanta wants as an email: the install's sender address (`OC_SMTP_FROM`).
pub fn creator(smtp_from: &str) -> String {
    smtp_from.parse::<lettre::message::Mailbox>().map_or_else(|_| "alerts@observecraft.local".to_string(), |m| m.email.to_string())
}

/// Vanta's priority for a severity.
fn priority(severity: &str) -> &'static str {
    match severity {
        "critical" => "P0",
        "high" => "P1",
        "medium" => "P2",
        "low" => "P3",
        _ => "P4",
    }
}

/// A finding as Vanta's `security_task`.
pub fn security_task(f: &Finding, ui: &str, creator: &str) -> Value {
    let title = if f.resource_name.is_empty() { format!("{}: {}", f.title, f.resource_uid) } else { format!("{}: {}", f.title, f.resource_name) };
    let open = f.evaluation == "fail" && !f.muted;
    let priority = priority(&f.severity);
    let mut labels: Vec<&str> = ["cloud-security", &f.provider, &f.account_id, &f.region, &f.resource_type, &f.check_id].into_iter().filter(|l| !l.is_empty()).collect();
    if f.muted {
        labels.push("muted");
    }
    let mut v = json!({
        "uniqueId": f.id,
        "displayName": title,
        "externalUrl": format!("{ui}/security/cloud/findings"),
        "taskId": f.id,
        "taskTitle": title,
        "createdTimestamp": rfc3339(f.first_seen),
        "updatedTimestamp": rfc3339(f.last_seen),
        "taskState": if open { "OPEN" } else { "CLOSED" },
        "priority": priority,
        "assignees": [],
        "creator": creator,
        "labels": labels,
    });
    if !open {
        // Muted: when; resolved: when the check passed again.
        let closed = if f.muted { f.muted_at.unwrap_or(f.changed_at) } else { f.changed_at };
        v["closedTimestamp"] = json!(rfc3339(closed));
    }
    v
}

/// A security signal as read for Vanta (times in epoch ms).
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Signal {
    pub id: String,
    pub rule_id: String,
    pub rule_type: String,
    pub title: String,
    pub severity: String,
    pub source: String,
    pub env: String,
    /// open | under_review | archived
    pub state: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub state_updated_at: Option<i64>,
}

/// A signal as Vanta's `security_task` (ids prefixed `signal:`, apart from the findings' UUIDs).
pub fn signal_task(s: &Signal, ui: &str, creator: &str) -> Value {
    let id = format!("signal:{}", s.id);
    let open = s.state != "archived";
    let labels: Vec<&str> = ["security-signal", &s.rule_type, &s.rule_id, &s.source, &s.env].into_iter().filter(|l| !l.is_empty()).collect();
    let mut v = json!({
        "uniqueId": id,
        "displayName": s.title,
        "externalUrl": format!("{ui}/security/signals?signal={}", url::form_urlencoded::byte_serialize(s.id.as_bytes()).collect::<String>()),
        "taskId": id,
        "taskTitle": s.title,
        "createdTimestamp": rfc3339(s.first_seen),
        "updatedTimestamp": rfc3339(s.last_seen),
        "taskState": if open { "OPEN" } else { "CLOSED" },
        "priority": priority(&s.severity),
        "assignees": [],
        "creator": creator,
        "labels": labels,
    });
    if !open {
        v["closedTimestamp"] = json!(rfc3339(s.state_updated_at.unwrap_or(s.last_seen)));
    }
    v
}

/// The org's security alerts to send: failing findings (open, or closed when muted), open and under-review signals, and
/// both resolved in the last [`RESOLVED_DAYS`] days; open ones first, most severe first.
pub async fn security_tasks(st: &AppState, org: &str) -> Result<Vec<Value>, sqlx::Error> {
    let rows: Vec<Finding> = sqlx::query_as(
        "SELECT f.id::text AS id, f.provider, f.account_id, f.check_id, COALESCE(NULLIF(c.title, ''), f.check_id) AS title, f.resource_uid, f.resource_name, f.resource_type, \
         f.region, f.severity, f.evaluation, f.muted, (extract(epoch FROM f.first_seen_at) * 1000)::bigint AS first_seen, (extract(epoch FROM f.last_seen_at) * 1000)::bigint AS last_seen, \
         (extract(epoch FROM f.evaluation_changed_at) * 1000)::bigint AS changed_at, (extract(epoch FROM f.muted_at) * 1000)::bigint AS muted_at \
         FROM posture_findings f LEFT JOIN posture_checks c ON c.provider = f.provider AND c.check_id = f.check_id \
         WHERE f.org_id = $1 AND (f.evaluation = 'fail' OR (f.evaluation_changed_at > f.first_seen_at AND f.evaluation_changed_at > now() - make_interval(days => $2))) \
         ORDER BY (f.evaluation = 'fail' AND NOT f.muted) DESC, CASE f.severity WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END, f.first_seen_at, f.id",
    )
    .bind(org)
    .bind(RESOLVED_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    let signals: Vec<Signal> = sqlx::query_as(
        "SELECT id, rule_id, rule_type, title, severity, source, env, state, (extract(epoch FROM first_seen) * 1000)::bigint AS first_seen, \
         (extract(epoch FROM last_seen) * 1000)::bigint AS last_seen, (extract(epoch FROM state_updated_at) * 1000)::bigint AS state_updated_at \
         FROM security_signals WHERE org_id = $1 AND NOT suppressed \
         AND (state <> 'archived' OR COALESCE(state_updated_at, last_seen) > now() - make_interval(days => $2)) ORDER BY first_seen, id",
    )
    .bind(org)
    .bind(RESOLVED_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    let (ui, from) = (&st.config.public_url, creator(&st.config.smtp_from));
    let mut tasks: Vec<Value> = rows.iter().map(|f| security_task(f, ui, &from)).chain(signals.iter().map(|s| signal_task(s, ui, &from))).collect();
    // Stable: within a state and priority, the findings' and signals' own order.
    tasks.sort_by_key(|t| (t["taskState"] != "OPEN", t["priority"].as_str().unwrap_or("P4").to_string()));
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: &str = "https://observecraft.example.com";
    const FROM: &str = "alerts@observecraft.example.com";

    fn finding() -> Finding {
        Finding {
            id: "00000000-0000-4000-8000-0000000000f1".into(),
            provider: "aws".into(),
            account_id: "123456789012".into(),
            check_id: "s3_bucket_public_access".into(),
            title: "Ensure S3 buckets block public access".into(),
            resource_uid: "arn:aws:s3:::example-shop-assets".into(),
            resource_name: "example-shop-assets".into(),
            resource_type: "AwsS3Bucket".into(),
            region: "us-east-1".into(),
            severity: "high".into(),
            evaluation: "fail".into(),
            muted: false,
            first_seen: 1_790_000_000_000,
            last_seen: 1_795_000_000_000,
            changed_at: 1_790_000_000_000,
            muted_at: None,
        }
    }

    #[test]
    fn a_failing_check_is_an_open_task() {
        assert_eq!(
            security_task(&finding(), UI, FROM),
            json!({
                "uniqueId": "00000000-0000-4000-8000-0000000000f1",
                "displayName": "Ensure S3 buckets block public access: example-shop-assets",
                "externalUrl": "https://observecraft.example.com/security/cloud/findings",
                "taskId": "00000000-0000-4000-8000-0000000000f1",
                "taskTitle": "Ensure S3 buckets block public access: example-shop-assets",
                "createdTimestamp": "2026-09-21T14:13:20Z",
                "updatedTimestamp": "2026-11-18T11:06:40Z",
                "taskState": "OPEN",
                "priority": "P1",
                "assignees": [],
                "creator": "alerts@observecraft.example.com",
                "labels": ["cloud-security", "aws", "123456789012", "us-east-1", "AwsS3Bucket", "s3_bucket_public_access"],
            })
        );
    }

    #[test]
    fn muted_and_passing_checks_are_closed_with_when() {
        let mut f = finding();
        f.muted = true;
        f.muted_at = Some(1_795_100_000_000);
        let t = security_task(&f, UI, FROM);
        assert_eq!((t["taskState"].as_str(), t["closedTimestamp"].as_str()), (Some("CLOSED"), Some("2026-11-19T14:53:20Z")));
        assert!(t["labels"].as_array().unwrap().contains(&json!("muted")));
        let mut f = finding();
        f.evaluation = "pass".into();
        f.changed_at = 1_795_000_000_000;
        let t = security_task(&f, UI, FROM);
        assert_eq!((t["taskState"].as_str(), t["closedTimestamp"].as_str()), (Some("CLOSED"), Some("2026-11-18T11:06:40Z")));
        // An open task has no closedTimestamp at all (Vanta refuses one).
        assert!(security_task(&finding(), UI, FROM).get("closedTimestamp").is_none());
    }

    #[test]
    fn priorities_follow_the_severity() {
        let mut f = finding();
        for (severity, priority) in [("critical", "P0"), ("high", "P1"), ("medium", "P2"), ("low", "P3"), ("informational", "P4")] {
            f.severity = severity.into();
            assert_eq!(security_task(&f, UI, FROM)["priority"], priority);
        }
        // Empty labels are left out.
        f.region = String::new();
        assert!(!security_task(&f, UI, FROM)["labels"].as_array().unwrap().contains(&json!("")));
    }

    #[test]
    fn the_creator_is_the_installs_sender_address() {
        // Vanta wants an email; the install's From address, else a fixed one.
        assert_eq!(creator("ObserveCraft <alerts@observecraft.example.com>"), "alerts@observecraft.example.com");
        assert_eq!(creator("alerts@observecraft.example.com"), "alerts@observecraft.example.com");
        assert_eq!(creator("not an address"), "alerts@observecraft.local");
    }

    fn signal() -> Signal {
        Signal {
            id: "AAAAAZNvMB9jX2V4YW1wbGU".into(),
            rule_id: "abc-def-ghi".into(),
            rule_type: "log_detection".into(),
            title: "Brute force attempt on storefront-api".into(),
            severity: "critical".into(),
            source: "nginx".into(),
            env: "prod".into(),
            state: "open".into(),
            first_seen: 1_790_000_000_000,
            last_seen: 1_795_000_000_000,
            state_updated_at: None,
        }
    }

    #[test]
    fn an_open_signal_is_an_open_task() {
        assert_eq!(
            signal_task(&signal(), UI, FROM),
            json!({
                "uniqueId": "signal:AAAAAZNvMB9jX2V4YW1wbGU",
                "displayName": "Brute force attempt on storefront-api",
                "externalUrl": "https://observecraft.example.com/security/signals?signal=AAAAAZNvMB9jX2V4YW1wbGU",
                "taskId": "signal:AAAAAZNvMB9jX2V4YW1wbGU",
                "taskTitle": "Brute force attempt on storefront-api",
                "createdTimestamp": "2026-09-21T14:13:20Z",
                "updatedTimestamp": "2026-11-18T11:06:40Z",
                "taskState": "OPEN",
                "priority": "P0",
                "assignees": [],
                "creator": "alerts@observecraft.example.com",
                "labels": ["security-signal", "log_detection", "abc-def-ghi", "nginx", "prod"],
            })
        );
        // Under review is still open; info is the lowest priority.
        let mut s = signal();
        s.state = "under_review".into();
        s.severity = "info".into();
        let t = signal_task(&s, UI, FROM);
        assert_eq!((t["taskState"].as_str(), t["priority"].as_str()), (Some("OPEN"), Some("P4")));
        assert!(t.get("closedTimestamp").is_none());
    }

    #[test]
    fn an_archived_signal_is_closed_when_it_was_archived() {
        let mut s = signal();
        s.state = "archived".into();
        s.state_updated_at = Some(1_795_100_000_000);
        let t = signal_task(&s, UI, FROM);
        assert_eq!((t["taskState"].as_str(), t["closedTimestamp"].as_str()), (Some("CLOSED"), Some("2026-11-19T14:53:20Z")));
        // Archived without a recorded time: when it was last seen.
        s.state_updated_at = None;
        assert_eq!(signal_task(&s, UI, FROM)["closedTimestamp"], "2026-11-18T11:06:40Z");
    }

    #[tokio::test]
    async fn open_and_recently_archived_signals_are_sent_with_the_findings_open_ones_first() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "vanta-signals").await, crate::test_support::new_org(st, "vanta-signals").await);
        sqlx::query("INSERT INTO posture_checks (provider, check_id, title, severity) VALUES ('aws', 'vanta_test_check', 'Example check', 'high') ON CONFLICT DO NOTHING").execute(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO posture_findings (org_id, provider, account_ref, account_id, check_id, resource_uid, resource_name, evaluation, severity, muted, first_seen_at, evaluation_changed_at) \
             VALUES ($1, 'aws', '00000000-0000-4000-8000-00000000a001'::uuid, '123456789012', 'vanta_test_check', 'bucket', 'bucket', 'fail', 'high', false, now() - interval '9 days', now() - interval '9 days')")
            .bind(&org)
            .execute(&st.postgres)
            .await
            .unwrap();
        for (o, id, severity, state, suppressed, updated) in [
            (&org, "s-medium", "medium", "open", false, "NULL"),
            (&org, "s-critical", "critical", "under_review", false, "now() - interval '1 day'"),
            (&org, "s-archived", "critical", "archived", false, "now() - interval '2 days'"),
            (&org, "s-archived-long-ago", "critical", "archived", false, "now() - interval '200 days'"),
            (&org, "s-suppressed", "critical", "open", true, "NULL"),
            (&other, "s-other-org", "critical", "open", false, "NULL"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO security_signals (org_id, id, rule_id, rule_name, title, severity, first_seen, last_seen, state, suppressed, state_updated_at) \
                 VALUES ($1, $2, 'rule-1', 'Rule', $2, $3, now() - interval '300 days', now() - interval '3 days', $4, $5, {updated})"
            )))
            .bind(o)
            .bind(id)
            .bind(severity)
            .bind(state)
            .bind(suppressed)
            .execute(&st.postgres)
            .await
            .unwrap();
        }
        let tasks = security_tasks(st, &org).await.unwrap();
        let sent: Vec<(&str, &str)> = tasks.iter().map(|t| (t["taskTitle"].as_str().unwrap(), t["taskState"].as_str().unwrap())).collect();
        assert_eq!(sent, [("s-critical", "OPEN"), ("Example check: bucket", "OPEN"), ("s-medium", "OPEN"), ("s-archived", "CLOSED")]);
        assert!(tasks.iter().all(|t| t["creator"] == creator(&st.config.smtp_from)));
    }

    #[tokio::test]
    async fn failing_muted_and_recently_resolved_findings_are_sent_open_ones_first() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "vanta-findings").await, crate::test_support::new_org(st, "vanta-findings").await);
        let account = "00000000-0000-4000-8000-00000000a001";
        sqlx::query("INSERT INTO posture_checks (provider, check_id, title, severity) VALUES ('aws', 'vanta_test_check', 'Example check', 'high') ON CONFLICT DO NOTHING").execute(&st.postgres).await.unwrap();
        for (o, uid, evaluation, severity, muted, changed) in [
            (&org, "open-low", "fail", "low", false, "now() - interval '5 days'"),
            (&org, "open-critical", "fail", "critical", false, "now() - interval '5 days'"),
            (&org, "muted", "fail", "high", true, "now() - interval '5 days'"),
            (&org, "fixed", "pass", "high", false, "now() - interval '1 day'"),
            (&org, "fixed-long-ago", "pass", "high", false, "now() - interval '200 days'"),
            (&org, "never-failed", "pass", "high", false, "now() - interval '10 days'"),
            (&other, "other-org", "fail", "critical", false, "now() - interval '5 days'"),
        ] {
            let first_seen = if uid == "never-failed" { changed.to_string() } else { "now() - interval '300 days'".to_string() };
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO posture_findings (org_id, provider, account_ref, account_id, check_id, resource_uid, resource_name, evaluation, severity, muted, first_seen_at, evaluation_changed_at) \
                 VALUES ($1, 'aws', $2::uuid, '123456789012', 'vanta_test_check', $3, $3, $4, $5, $6, {first_seen}, {changed})"
            )))
            .bind(o)
            .bind(account)
            .bind(uid)
            .bind(evaluation)
            .bind(severity)
            .bind(muted)
            .execute(&st.postgres)
            .await
            .unwrap();
        }
        let tasks = security_tasks(st, &org).await.unwrap();
        let sent: Vec<(&str, &str)> = tasks.iter().map(|t| (t["taskTitle"].as_str().unwrap(), t["taskState"].as_str().unwrap())).collect();
        assert_eq!(sent, [("Example check: open-critical", "OPEN"), ("Example check: open-low", "OPEN"), ("Example check: muted", "CLOSED"), ("Example check: fixed", "CLOSED")]);
    }
}
