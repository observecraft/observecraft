//! Posture scans and findings in Postgres (`posture_scans`, `posture_checks`, `posture_findings`, migration 0088).
//!
//! - **Queue** ([`plan`]): a scan per account whose posture collection is on, every
//!   `OC_POSTURE_SCAN_INTERVAL_HOURS`; a failed scan is retried after an hour. At most one queued or running scan per
//!   account (a partial unique index).
//! - **Claim** ([`claim`]): the oldest queued scan, `FOR UPDATE SKIP LOCKED`, with a lease the scanner renews
//!   ([`heartbeat`]) while Prowler runs; a scan whose lease lapses fails ([`expire`]).
//! - **Results** ([`store_results`]): the checks' descriptions are kept (global), each evaluation upserted for its
//!   (account, check, resource), and the account's rows the scan didn't report removed. Evaluations of another account
//!   than the scan's are dropped, so a scanner can only write the account it was given.

use std::collections::HashMap;

use serde_json::Value;
use sqlx::PgPool;

use super::ocsf::Results;

/// How long a claim holds before the scanner must renew it.
pub const LEASE_SECS: i64 = 600;

/// A claimed scan.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Claimed {
    pub id: String,
    pub org_id: String,
    pub provider: String,
    pub account_ref: String,
    pub account_id: String,
}

/// Queues the scans that are due; returns how many.
pub async fn plan(pg: &PgPool, interval_hours: i64) -> Result<u64, sqlx::Error> {
    let queued = sqlx::query(
        "INSERT INTO posture_scans (org_id, provider, account_ref, account_id) \
         SELECT a.org_id, 'aws', a.id, a.aws_account_id FROM aws_accounts a JOIN orgs o ON o.id = a.org_id AND o.status = 'active' \
         WHERE COALESCE((a.resources_config->>'cloud_security_posture_management_collection')::boolean, false) \
           AND NOT EXISTS (SELECT 1 FROM posture_scans s WHERE s.account_ref = a.id AND ( \
                 s.status IN ('queued', 'running') \
              OR (s.status = 'done' AND s.finished_at > now() - make_interval(hours => $1::int)) \
              OR (s.status = 'failed' AND s.finished_at > now() - interval '1 hour'))) \
         ON CONFLICT DO NOTHING",
    )
    .bind(interval_hours as i32)
    .execute(pg)
    .await?
    .rows_affected();
    // Scans of accounts that were removed or turned off, and old history.
    sqlx::query(
        "DELETE FROM posture_scans s WHERE (s.status = 'queued' AND NOT EXISTS (SELECT 1 FROM aws_accounts a WHERE a.id = s.account_ref \
            AND COALESCE((a.resources_config->>'cloud_security_posture_management_collection')::boolean, false))) \
         OR s.queued_at < now() - interval '30 days'",
    )
    .execute(pg)
    .await?;
    sqlx::query("DELETE FROM posture_findings f WHERE NOT EXISTS (SELECT 1 FROM aws_accounts a WHERE a.org_id = f.org_id AND a.id = f.account_ref)").execute(pg).await?;
    Ok(queued)
}

/// Fails the running scans whose scanner stopped renewing its lease.
pub async fn expire(pg: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE posture_scans SET status = 'failed', finished_at = now(), error = 'The scanner stopped responding' WHERE status = 'running' AND lease_until < now()")
        .execute(pg)
        .await?
        .rows_affected())
}

/// Takes the oldest queued scan for `scanner`.
pub async fn claim(pg: &PgPool, scanner: &str) -> Result<Option<Claimed>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE posture_scans SET status = 'running', scanner = $1, started_at = now(), lease_until = now() + make_interval(secs => $2::double precision) \
         WHERE id = (SELECT id FROM posture_scans WHERE status = 'queued' ORDER BY queued_at FOR UPDATE SKIP LOCKED LIMIT 1) \
         RETURNING id::text, org_id, provider, account_ref::text, account_id",
    )
    .bind(scanner)
    .bind(LEASE_SECS as f64)
    .fetch_optional(pg)
    .await
}

/// Renews a running scan's lease; false when the scan isn't running any more (expired, account removed).
pub async fn heartbeat(pg: &PgPool, id: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("UPDATE posture_scans SET lease_until = now() + make_interval(secs => $2::double precision) WHERE id = $1::uuid AND status = 'running'")
        .bind(id)
        .bind(LEASE_SECS as f64)
        .execute(pg)
        .await?
        .rows_affected()
        == 1)
}

pub async fn fail(pg: &PgPool, id: &str, error: &str) -> Result<(), sqlx::Error> {
    let error: String = error.chars().take(2000).collect();
    sqlx::query("UPDATE posture_scans SET status = 'failed', finished_at = now(), error = $2 WHERE id = $1::uuid AND status = 'running'").bind(id).bind(error).execute(pg).await?;
    Ok(())
}

/// A running scan (None when it isn't running).
pub async fn running(pg: &PgPool, id: &str) -> Result<Option<Claimed>, sqlx::Error> {
    sqlx::query_as("SELECT id::text, org_id, provider, account_ref::text, account_id FROM posture_scans WHERE id = $1::uuid AND status = 'running'").bind(id).fetch_optional(pg).await
}

/// Stores a scan's results and closes it; returns (passed, failed) of the account's evaluations.
pub async fn store_results(pg: &PgPool, scan: &Claimed, results: &Results) -> Result<(i32, i32), sqlx::Error> {
    let mut tx = pg.begin().await?;
    for c in results.checks.values() {
        sqlx::query(
            "INSERT INTO posture_checks (provider, check_id, title, severity, service, resource_type, description, risk, remediation, reference_urls, compliance, tool_version, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now()) \
             ON CONFLICT (provider, check_id) DO UPDATE SET title = EXCLUDED.title, severity = EXCLUDED.severity, service = EXCLUDED.service, \
               resource_type = EXCLUDED.resource_type, description = EXCLUDED.description, risk = EXCLUDED.risk, remediation = EXCLUDED.remediation, \
               reference_urls = EXCLUDED.reference_urls, compliance = EXCLUDED.compliance, tool_version = EXCLUDED.tool_version, updated_at = now()",
        )
        .bind(&scan.provider)
        .bind(&c.check_id)
        .bind(&c.title)
        .bind(&c.severity)
        .bind(&c.service)
        .bind(&c.resource_type)
        .bind(&c.description)
        .bind(&c.risk)
        .bind(&c.remediation)
        .bind(serde_json::to_value(&c.reference_urls).unwrap_or(Value::Null))
        .bind(serde_json::to_value(&c.compliance).unwrap_or(Value::Null))
        .bind(&results.tool_version)
        .execute(&mut *tx)
        .await?;
    }
    // One row per (check, resource): Prowler may report a resource twice (regional and global views).
    let mut rows: HashMap<(&str, &str), &super::ocsf::Evaluation> = HashMap::new();
    for e in results.evaluations.iter().filter(|e| e.account_id.is_empty() || e.account_id == scan.account_id) {
        rows.entry((e.check_id.as_str(), e.resource_uid.as_str())).and_modify(|cur| if !e.pass { *cur = e }).or_insert(e);
    }
    let (mut passed, mut failed) = (0, 0);
    for chunk in rows.values().collect::<Vec<_>>().chunks(1000) {
        let mut cols: [Vec<String>; 8] = Default::default();
        let mut tags: Vec<Value> = vec![];
        for e in chunk {
            if e.pass {
                passed += 1;
            } else {
                failed += 1;
            }
            for (col, v) in cols.iter_mut().zip([&e.check_id, &e.resource_uid, &e.resource_name, &e.resource_type, &e.region, &e.severity, &e.detail]) {
                col.push(v.clone());
            }
            cols[7].push(if e.pass { "pass" } else { "fail" }.into());
            tags.push(serde_json::to_value(&e.tags).unwrap_or(Value::Null));
        }
        sqlx::query(
            "INSERT INTO posture_findings AS f (org_id, provider, account_ref, account_id, check_id, resource_uid, resource_name, resource_type, region, severity, detail, evaluation, resource_tags, last_scan) \
             SELECT $1, $2, $3::uuid, $4, t.check_id, t.resource_uid, t.resource_name, t.resource_type, t.region, t.severity, t.detail, t.evaluation, t.tags, $5::uuid \
             FROM UNNEST($6::text[], $7::text[], $8::text[], $9::text[], $10::text[], $11::text[], $12::text[], $13::text[], $14::jsonb[]) \
               AS t(check_id, resource_uid, resource_name, resource_type, region, severity, detail, evaluation, tags) \
             ON CONFLICT (org_id, account_ref, check_id, resource_uid) DO UPDATE SET \
               resource_name = EXCLUDED.resource_name, resource_type = EXCLUDED.resource_type, region = EXCLUDED.region, severity = EXCLUDED.severity, \
               detail = EXCLUDED.detail, resource_tags = EXCLUDED.resource_tags, last_seen_at = now(), last_scan = EXCLUDED.last_scan, \
               evaluation_changed_at = CASE WHEN f.evaluation <> EXCLUDED.evaluation THEN now() ELSE f.evaluation_changed_at END, \
               evaluation = EXCLUDED.evaluation",
        )
        .bind(&scan.org_id)
        .bind(&scan.provider)
        .bind(&scan.account_ref)
        .bind(&scan.account_id)
        .bind(&scan.id)
        .bind(&cols[0])
        .bind(&cols[1])
        .bind(&cols[2])
        .bind(&cols[3])
        .bind(&cols[4])
        .bind(&cols[5])
        .bind(&cols[6])
        .bind(&cols[7])
        .bind(&tags)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM posture_findings WHERE org_id = $1 AND account_ref = $2::uuid AND last_scan IS DISTINCT FROM $3::uuid").bind(&scan.org_id).bind(&scan.account_ref).bind(&scan.id).execute(&mut *tx).await?;
    sqlx::query("UPDATE posture_scans SET status = 'done', finished_at = now(), error = NULL, tool_version = $2, passed = $3, failed = $4 WHERE id = $1::uuid")
        .bind(&scan.id)
        .bind(&results.tool_version)
        .bind(passed)
        .bind(failed)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((passed, failed))
}

// ================================================================ reading

/// A finding with its check's description.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct FindingRow {
    pub id: String,
    pub provider: String,
    pub account_id: String,
    pub check_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub risk: Option<String>,
    pub remediation: Option<String>,
    pub service: Option<String>,
    pub compliance: Option<Value>,
    pub reference_urls: Option<Value>,
    pub resource_uid: String,
    pub resource_name: String,
    pub resource_type: String,
    pub region: String,
    pub evaluation: String,
    pub severity: String,
    pub detail: String,
    pub resource_tags: Value,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub evaluation_changed_at: i64,
    /// Muted and not expired.
    pub muted: bool,
    pub mute_reason: Option<String>,
    pub mute_description: Option<String>,
    pub mute_expire_at: Option<i64>,
    pub muted_by: Option<String>,
    pub muted_at: Option<i64>,
}

const SELECT: &str = "SELECT f.id::text, f.provider, f.account_id, f.check_id, c.title, c.description, c.risk, c.remediation, c.service, \
    c.compliance, c.reference_urls, f.resource_uid, f.resource_name, f.resource_type, f.region, f.evaluation, f.severity, f.detail, f.resource_tags, \
    (extract(epoch FROM f.first_seen_at) * 1000)::bigint AS first_seen_at, (extract(epoch FROM f.last_seen_at) * 1000)::bigint AS last_seen_at, \
    (extract(epoch FROM f.evaluation_changed_at) * 1000)::bigint AS evaluation_changed_at, \
    (f.muted AND (f.mute_expire_at IS NULL OR f.mute_expire_at > now())) AS muted, f.mute_reason, f.mute_description, \
    (extract(epoch FROM f.mute_expire_at) * 1000)::bigint AS mute_expire_at, f.muted_by, (extract(epoch FROM f.muted_at) * 1000)::bigint AS muted_at \
    FROM posture_findings f LEFT JOIN posture_checks c ON c.provider = f.provider AND c.check_id = f.check_id";

/// The org's findings, newest evaluation change first; `failing_only` leaves out passes.
pub async fn findings(pg: &PgPool, org: &str, failing_only: bool) -> Result<Vec<FindingRow>, sqlx::Error> {
    let sql = format!("{SELECT} WHERE f.org_id = $1 AND ($2 = false OR f.evaluation = 'fail') ORDER BY f.evaluation_changed_at DESC, f.id");
    sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(org).bind(failing_only).fetch_all(pg).await
}

/// Which of `ids` (finding row ids) belong to the org.
pub async fn existing(pg: &PgPool, org: &str, ids: &[String]) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT id::text FROM posture_findings WHERE org_id = $1 AND id::text = ANY($2)").bind(org).bind(ids).fetch_all(pg).await
}

/// Mutes or unmutes findings of the org.
pub async fn set_mute(pg: &PgPool, org: &str, ids: &[String], mute: Option<(&str, &str, Option<i64>)>, actor: &str) -> Result<u64, sqlx::Error> {
    let q = match mute {
        Some((reason, description, expire_ms)) => sqlx::query(
            "UPDATE posture_findings SET muted = true, mute_reason = $3, mute_description = $4, \
               mute_expire_at = CASE WHEN $5::bigint IS NULL THEN NULL ELSE to_timestamp($5::bigint / 1000.0) END, muted_by = $6, muted_at = now() \
             WHERE org_id = $1 AND id::text = ANY($2)",
        )
        .bind(org)
        .bind(ids)
        .bind(reason)
        .bind(description)
        .bind(expire_ms)
        .bind(actor),
        None => sqlx::query(
            "UPDATE posture_findings SET muted = false, mute_reason = NULL, mute_description = NULL, mute_expire_at = NULL, muted_by = $3, muted_at = now() \
             WHERE org_id = $1 AND id::text = ANY($2)",
        )
        .bind(org)
        .bind(ids)
        .bind(actor),
    };
    Ok(q.execute(pg).await?.rows_affected())
}

/// The latest scan of each of the org's accounts.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ScanRow {
    pub id: String,
    pub account_ref: String,
    pub status: String,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
    pub tool_version: Option<String>,
    pub passed: Option<i32>,
    pub failed: Option<i32>,
}

pub async fn latest_scans(pg: &PgPool, org: &str) -> Result<Vec<ScanRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT DISTINCT ON (account_ref) id::text, account_ref::text, status, \
           (extract(epoch FROM queued_at) * 1000)::bigint AS queued_at, (extract(epoch FROM started_at) * 1000)::bigint AS started_at, \
           (extract(epoch FROM finished_at) * 1000)::bigint AS finished_at, error, tool_version, passed, failed \
         FROM posture_scans WHERE org_id = $1 ORDER BY account_ref, queued_at DESC",
    )
    .bind(org)
    .fetch_all(pg)
    .await
}

/// Queues a scan of one of the org's accounts now (unless one is queued or running). False when the account isn't
/// the org's or doesn't have posture collection on.
pub async fn scan_now(pg: &PgPool, org: &str, account_ref: &str) -> Result<bool, sqlx::Error> {
    let on: Option<(String, bool)> = sqlx::query_as(
        "SELECT aws_account_id, COALESCE((resources_config->>'cloud_security_posture_management_collection')::boolean, false) FROM aws_accounts WHERE org_id = $1 AND id::text = $2",
    )
    .bind(org)
    .bind(account_ref)
    .fetch_optional(pg)
    .await?;
    let Some((account_id, true)) = on else { return Ok(false) };
    sqlx::query("INSERT INTO posture_scans (org_id, provider, account_ref, account_id) VALUES ($1, 'aws', $2::uuid, $3) ON CONFLICT DO NOTHING").bind(org).bind(account_ref).bind(account_id).execute(pg).await?;
    Ok(true)
}
