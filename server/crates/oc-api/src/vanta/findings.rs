//! Cloud Security's misconfigurations (the account scans' failed checks, `posture_findings`) as Vanta `security_task`
//! resources, as Vanta reads Cloud Security's findings from Datadog for its security alert SLA tests: open while the
//! check fails, closed when it passes again or was muted, with its priority from the severity. Checks that never failed
//! aren't findings; resolved ones are kept [`RESOLVED_DAYS`] days, so Vanta sees when they were fixed.

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

/// A finding as Vanta's `security_task`.
pub fn security_task(f: &Finding, ui: &str) -> Value {
    let rfc3339 = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let title = if f.resource_name.is_empty() { format!("{}: {}", f.title, f.resource_uid) } else { format!("{}: {}", f.title, f.resource_name) };
    let open = f.evaluation == "fail" && !f.muted;
    let priority = match f.severity.as_str() {
        "critical" => "P0",
        "high" => "P1",
        "medium" => "P2",
        "low" => "P3",
        _ => "P4",
    };
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
        "creator": "ObserveCraft Cloud Security",
        "labels": labels,
    });
    if !open {
        // Muted: when; resolved: when the check passed again.
        let closed = if f.muted { f.muted_at.unwrap_or(f.changed_at) } else { f.changed_at };
        v["closedTimestamp"] = json!(rfc3339(closed));
    }
    v
}

/// The org's findings to send: failing ones (open, or closed when muted) and those resolved in the last
/// [`RESOLVED_DAYS`] days; open ones first, most severe first.
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
    Ok(rows.iter().map(|f| security_task(f, &st.config.public_url)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: &str = "https://observecraft.example.com";

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
            security_task(&finding(), UI),
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
                "creator": "ObserveCraft Cloud Security",
                "labels": ["cloud-security", "aws", "123456789012", "us-east-1", "AwsS3Bucket", "s3_bucket_public_access"],
            })
        );
    }

    #[test]
    fn muted_and_passing_checks_are_closed_with_when() {
        let mut f = finding();
        f.muted = true;
        f.muted_at = Some(1_795_100_000_000);
        let t = security_task(&f, UI);
        assert_eq!((t["taskState"].as_str(), t["closedTimestamp"].as_str()), (Some("CLOSED"), Some("2026-11-19T14:53:20Z")));
        assert!(t["labels"].as_array().unwrap().contains(&json!("muted")));
        let mut f = finding();
        f.evaluation = "pass".into();
        f.changed_at = 1_795_000_000_000;
        let t = security_task(&f, UI);
        assert_eq!((t["taskState"].as_str(), t["closedTimestamp"].as_str()), (Some("CLOSED"), Some("2026-11-18T11:06:40Z")));
        // An open task has no closedTimestamp at all (Vanta refuses one).
        assert!(security_task(&finding(), UI).get("closedTimestamp").is_none());
    }

    #[test]
    fn priorities_follow_the_severity() {
        let mut f = finding();
        for (severity, priority) in [("critical", "P0"), ("high", "P1"), ("medium", "P2"), ("low", "P3"), ("informational", "P4")] {
            f.severity = severity.into();
            assert_eq!(security_task(&f, UI)["priority"], priority);
        }
        // Empty labels are left out.
        f.region = String::new();
        assert!(!security_task(&f, UI)["labels"].as_array().unwrap().contains(&json!("")));
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
