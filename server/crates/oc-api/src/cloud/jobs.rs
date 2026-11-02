//! The cloud crawlers' jobs (`cloud_crawl_jobs`), shared by AWS, Azure and GCP: one job per provider,
//! account and scope (an AWS region, an Azure subscription, a GCP project).
//!
//! - **Planning** ([`plan`]): every minute, one server per provider (an advisory transaction lock)
//!   brings the jobs in line with the accounts. A provider's [`Crawler::desired`] lists either every
//!   job (AWS regions) or one discovery job per account (scope `""`), whose run lists the account's
//!   subscriptions or projects and becomes their jobs ([`Outcome::Discovered`]).
//! - **Claiming** ([`claim`]): due jobs are taken with `FOR UPDATE SKIP LOCKED` and held by a lease,
//!   so adding servers adds crawl capacity and a job runs once.
//! - **Recording** ([`record`]): after a run the watermark (`collected_until`, advanced only once Kafka
//!   acknowledged the window), the next run, or the error (the account's issue) and its backoff.
//! - Points go to Kafka through a [`MetricSink`] with the org's headers ([`org_headers`]), as the
//!   intake sends them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

/// Where the points go: the Kafka topic the collector reads OTLP JSON metrics from.
#[async_trait::async_trait]
pub trait MetricSink: Send + Sync {
    /// Writes one record and waits for the acknowledgement.
    async fn publish(&self, org: &str, headers: &[(String, String)], body: Vec<u8>) -> Result<(), String>;
}

/// How long a claimed job is held.
const LEASE_SECS: f64 = 600.0;
/// How often a discovery job runs.
pub const DISCOVERY_SECS: i64 = 3600;

/// A job: which provider, org, account (its id in the provider's accounts table) and scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobKey {
    pub provider: String,
    pub org_id: String,
    pub account_id: String,
    pub scope: String,
    /// The watermark, epoch ms.
    pub collected_until: Option<i64>,
}

/// When a failed job runs again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backoff {
    /// Doubling from a minute up to the interval (throttling, transient errors).
    Exponential,
    /// An interval (configuration problems: the customer has to act).
    Interval,
    /// A day (a disabled region or project).
    Day,
}

/// A run's problem: the account's issue.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    /// access_denied | auth | throttled | disabled | other
    pub kind: &'static str,
    pub message: String,
    /// The permission the provider refused, when it names it.
    pub missing_action: Option<String>,
    pub backoff: Backoff,
}

/// What a run did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Points up to `end_ms` were published; a warning (e.g. tags that couldn't be read) is kept as the issue.
    Collected { end_ms: i64, metrics: usize, points: usize, warning: Option<Issue> },
    /// A discovery job's scopes (subscriptions, projects): they become the account's jobs.
    Discovered(Vec<String>),
    /// Nothing to read yet.
    Idle,
    Failed(Issue),
}

/// A provider's crawler.
#[async_trait::async_trait]
pub trait Crawler: Send + Sync {
    fn provider(&self) -> &'static str;
    /// The jobs the accounts call for: `(org, account, scope)`; `""` is an account's discovery job.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error>;
    /// Runs one job (`now_ms`: the time the window ends at, minus the provider's lag).
    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error>;
}

fn lock_key(provider: &str) -> i64 {
    7321 + provider.bytes().map(i64::from).sum::<i64>()
}

/// Brings a provider's jobs in line with [`Crawler::desired`]: the number of jobs, or None when another
/// server is planning. Jobs of accounts no longer desired go; a desired account with listed scopes keeps
/// only those; an account with only a discovery job keeps the scopes its discovery found.
pub async fn plan(st: &AppState, provider: &str, desired: &[(String, String, String)]) -> Result<Option<usize>, sqlx::Error> {
    let mut tx = st.postgres.begin().await?;
    if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_xact_lock($1)").bind(lock_key(provider)).fetch_one(&mut *tx).await? {
        return Ok(None);
    }
    let orgs: Vec<&str> = desired.iter().map(|d| d.0.as_str()).collect();
    let accounts: Vec<&str> = desired.iter().map(|d| d.1.as_str()).collect();
    let scopes: Vec<&str> = desired.iter().map(|d| d.2.as_str()).collect();
    sqlx::query(
        "INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, o, a::uuid, s FROM unnest($2::text[], $3::text[], $4::text[]) AS d(o, a, s) \
         ON CONFLICT DO NOTHING",
    )
    .bind(provider)
    .bind(&orgs)
    .bind(&accounts)
    .bind(&scopes)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH d AS (SELECT o, a::uuid AS a, s FROM unnest($2::text[], $3::text[], $4::text[]) AS d(o, a, s)) \
         DELETE FROM cloud_crawl_jobs j WHERE j.provider = $1 AND ( \
           NOT EXISTS (SELECT 1 FROM d WHERE d.o = j.org_id AND d.a = j.account_id) \
           OR (NOT EXISTS (SELECT 1 FROM d WHERE d.o = j.org_id AND d.a = j.account_id AND d.s = j.scope) \
               AND (j.scope = '' OR EXISTS (SELECT 1 FROM d WHERE d.o = j.org_id AND d.a = j.account_id AND d.s <> ''))))",
    )
    .bind(provider)
    .bind(&orgs)
    .bind(&accounts)
    .bind(&scopes)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(desired.len()))
}

/// A discovery's result: the account's scope jobs become exactly `scopes`.
async fn sync_scopes(st: &AppState, job: &JobKey, scopes: &[String]) -> Result<(), sqlx::Error> {
    let mut tx = st.postgres.begin().await?;
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, $2, $3::uuid, s FROM unnest($4::text[]) AS s ON CONFLICT DO NOTHING")
        .bind(&job.provider)
        .bind(&job.org_id)
        .bind(&job.account_id)
        .bind(scopes)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid AND scope <> '' AND NOT (scope = ANY($4))")
        .bind(&job.provider)
        .bind(&job.org_id)
        .bind(&job.account_id)
        .bind(scopes)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Claims up to `limit` due jobs of a provider (held for the lease).
pub async fn claim(st: &AppState, provider: &str, limit: i64) -> Result<Vec<JobKey>, sqlx::Error> {
    let rows: Vec<(String, String, String, Option<i64>)> = sqlx::query_as(
        "WITH due AS (SELECT j.provider, j.org_id, j.account_id, j.scope FROM cloud_crawl_jobs j JOIN orgs o ON o.id = j.org_id AND o.status = 'active' \
           WHERE j.provider = $1 AND j.next_run_at <= now() AND (j.claimed_until IS NULL OR j.claimed_until < now()) ORDER BY j.next_run_at LIMIT $2 FOR UPDATE OF j SKIP LOCKED) \
         UPDATE cloud_crawl_jobs j SET claimed_until = now() + make_interval(secs => $3), last_run_at = now() FROM due \
         WHERE j.provider = due.provider AND j.org_id = due.org_id AND j.account_id = due.account_id AND j.scope = due.scope \
         RETURNING j.org_id, j.account_id::text, j.scope, (extract(epoch FROM j.collected_until) * 1000)::bigint",
    )
    .bind(provider)
    .bind(limit)
    .bind(LEASE_SECS)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().map(|(org_id, account_id, scope, collected_until)| JobKey { provider: provider.to_string(), org_id, account_id, scope, collected_until }).collect())
}

fn retry_secs(backoff: Backoff, failures: i32, interval_secs: i64) -> i64 {
    match backoff {
        Backoff::Day => 86_400,
        Backoff::Interval => interval_secs,
        Backoff::Exponential => (60i64 << failures.clamp(0, 6)).min(interval_secs),
    }
}

const KEY: &str = "WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid AND scope = $4";

/// Records a run: the watermark and the next run after a success, the issue and its backoff after a failure.
pub async fn record(st: &AppState, job: &JobKey, outcome: &Outcome, interval_secs: i64) -> Result<(), sqlx::Error> {
    match outcome {
        Outcome::Idle => {
            sqlx::query(&format!("UPDATE cloud_crawl_jobs SET claimed_until = NULL, next_run_at = now() + make_interval(secs => 60) {KEY}"))
                .bind(&job.provider)
                .bind(&job.org_id)
                .bind(&job.account_id)
                .bind(&job.scope)
                .execute(&st.postgres)
                .await?;
        }
        Outcome::Discovered(scopes) => {
            sync_scopes(st, job, scopes).await?;
            sqlx::query(&format!(
                "UPDATE cloud_crawl_jobs SET claimed_until = NULL, last_success_at = now(), failures = 0, error_kind = NULL, error_message = NULL, missing_action = NULL, \
                 error_at = NULL, metrics_last_run = $5, next_run_at = now() + make_interval(secs => $6) {KEY}"
            ))
            .bind(&job.provider)
            .bind(&job.org_id)
            .bind(&job.account_id)
            .bind(&job.scope)
            .bind(scopes.len() as i32)
            .bind(DISCOVERY_SECS as f64)
            .execute(&st.postgres)
            .await?;
        }
        Outcome::Collected { end_ms, metrics, points, warning } => {
            sqlx::query(&format!(
                "UPDATE cloud_crawl_jobs SET claimed_until = NULL, collected_until = to_timestamp($5::double precision / 1000), last_success_at = now(), failures = 0, \
                 next_run_at = now() + make_interval(secs => $6), metrics_last_run = $7, points_last_run = $8, error_kind = $9, error_message = $10, missing_action = $11, \
                 error_at = CASE WHEN $9::text IS NULL THEN NULL ELSE now() END {KEY}"
            ))
            .bind(&job.provider)
            .bind(&job.org_id)
            .bind(&job.account_id)
            .bind(&job.scope)
            .bind(*end_ms as f64)
            .bind(interval_secs as f64)
            .bind(*metrics as i32)
            .bind(*points as i32)
            .bind(warning.as_ref().map(|w| w.kind))
            .bind(warning.as_ref().map(|w| w.message.clone()))
            .bind(warning.as_ref().and_then(|w| w.missing_action.clone()))
            .execute(&st.postgres)
            .await?;
        }
        Outcome::Failed(issue) => {
            let failures: i32 = sqlx::query_scalar(&format!("SELECT failures FROM cloud_crawl_jobs {KEY}"))
                .bind(&job.provider)
                .bind(&job.org_id)
                .bind(&job.account_id)
                .bind(&job.scope)
                .fetch_optional(&st.postgres)
                .await?
                .unwrap_or(0);
            sqlx::query(&format!(
                "UPDATE cloud_crawl_jobs SET claimed_until = NULL, failures = failures + 1, error_kind = $5, error_message = $6, missing_action = $7, error_at = now(), \
                 next_run_at = now() + make_interval(secs => $8) {KEY}"
            ))
            .bind(&job.provider)
            .bind(&job.org_id)
            .bind(&job.account_id)
            .bind(&job.scope)
            .bind(issue.kind)
            .bind(&issue.message)
            .bind(&issue.missing_action)
            .bind(retry_secs(issue.backoff, failures, if job.scope.is_empty() { DISCOVERY_SECS } else { interval_secs }) as f64)
            .execute(&st.postgres)
            .await?;
        }
    }
    Ok(())
}

/// The last crawl problem of a scope.
#[derive(Serialize, ToSchema, Debug)]
pub struct CrawlIssue {
    /// access_denied | auth | throttled | disabled | other
    pub kind: String,
    pub message: String,
    /// The permission the provider refused, when it names it.
    pub missing_action: Option<String>,
    /// Epoch ms.
    pub at: i64,
}

/// One scope's crawl (scope `""`: the account's discovery of its subscriptions or projects).
#[derive(Serialize, ToSchema, Debug)]
pub struct ScopeStatus {
    pub scope: String,
    /// Epoch ms of the last successful run.
    pub last_success_at: Option<i64>,
    /// Epoch ms up to which metrics are collected.
    pub collected_until: Option<i64>,
    /// Metrics and points of the last successful run (a discovery: the scopes found).
    pub metrics: i32,
    pub points: i32,
    pub error: Option<CrawlIssue>,
}

type StatusRow = (String, Option<i64>, Option<i64>, i32, i32, Option<String>, Option<String>, Option<String>, Option<i64>);

/// An account's jobs, by scope.
pub async fn status(st: &AppState, provider: &str, org: &str, account: &str) -> Result<Vec<ScopeStatus>, sqlx::Error> {
    let ms = |c: &str| format!("(extract(epoch FROM {c}) * 1000)::bigint");
    let rows: Vec<StatusRow> = sqlx::query_as(&format!(
        "SELECT scope, {}, {}, metrics_last_run, points_last_run, error_kind, error_message, missing_action, {} FROM cloud_crawl_jobs \
         WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid ORDER BY scope",
        ms("last_success_at"),
        ms("collected_until"),
        ms("error_at")
    ))
    .bind(provider)
    .bind(org)
    .bind(account)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(scope, last_success_at, collected_until, metrics, points, kind, message, missing_action, at)| ScopeStatus {
            scope,
            last_success_at,
            collected_until,
            metrics,
            points,
            error: kind.map(|kind| CrawlIssue { kind, message: message.unwrap_or_default(), missing_action, at: at.unwrap_or_default() }),
        })
        .collect())
}

/// The org's headers on every record, as the intake sends them (the collector reads the org and the
/// retention from them).
pub async fn org_headers(st: &AppState, org: &str) -> Result<Vec<(String, String)>, sqlx::Error> {
    let r: Option<(i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT COALESCE(span_retention_days, 0), COALESCE(metrics_retention_days, 0), COALESCE(metrics_5m_days, 0), COALESCE(metrics_1h_days, 0) FROM org_data_settings WHERE org_id = $1",
    )
    .bind(org)
    .fetch_optional(&st.postgres)
    .await?;
    let (span, metrics, m5, h1) = r.unwrap_or_default();
    Ok(vec![
        ("x-oc-org".into(), org.to_string()),
        ("x-oc-retention-days".into(), span.to_string()),
        ("x-oc-metrics-retention-days".into(), metrics.to_string()),
        ("x-oc-metrics-retention-5m-days".into(), m5.to_string()),
        ("x-oc-metrics-retention-1h-days".into(), h1.to_string()),
    ])
}

/// A crawler's loop: plans every minute, claims up to `concurrency` jobs at a time and runs them.
pub async fn run(st: AppState, crawler: Arc<dyn Crawler>, interval_secs: i64, concurrency: i64) {
    let provider = crawler.provider();
    let mut planned_at: Option<Instant> = None;
    loop {
        if planned_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            let planned = match crawler.desired(&st).await {
                Ok(desired) => plan(&st, provider, &desired).await,
                Err(e) => Err(e),
            };
            if let Err(e) = planned {
                tracing::warn!(provider, error = %e, "cloud crawler: planning failed");
            }
            planned_at = Some(Instant::now());
        }
        let jobs = claim(&st, provider, concurrency).await.unwrap_or_else(|e| {
            tracing::warn!(provider, error = %e, "cloud crawler: claiming jobs failed");
            vec![]
        });
        if jobs.is_empty() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let runs = jobs.iter().map(|job| async {
            let outcome = crawler.run(&st, job, now_ms).await?;
            record(&st, job, &outcome, interval_secs).await?;
            Ok::<_, sqlx::Error>(outcome)
        });
        for (job, result) in jobs.iter().zip(futures::future::join_all(runs).await) {
            match result {
                Ok(Outcome::Failed(issue)) => tracing::info!(provider, org = %job.org_id, scope = %job.scope, error = %issue.message, "cloud crawl failed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(provider, error = %e, "cloud crawler: a run failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff() {
        assert_eq!(retry_secs(Backoff::Exponential, 0, 600), 60);
        assert_eq!(retry_secs(Backoff::Exponential, 3, 600), 480);
        assert_eq!(retry_secs(Backoff::Exponential, 6, 600), 600);
        assert_eq!(retry_secs(Backoff::Day, 0, 600), 86_400);
        assert_eq!(retry_secs(Backoff::Interval, 4, 600), 600);
    }

    fn d(o: &str, a: &str, s: &str) -> (String, String, String) {
        (o.into(), a.into(), s.into())
    }

    // Each test plans its own provider: planning a provider replaces all of its jobs.
    const SCOPES: &str = "test-scopes";
    const RECORD: &str = "test-record";

    async fn scopes(st: &AppState, org: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 ORDER BY scope").bind(SCOPES).bind(org).fetch_all(&st.postgres).await.unwrap()
    }

    async fn planned(st: &AppState, provider: &str, desired: &[(String, String, String)]) {
        while plan(st, provider, desired).await.unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn listed_scopes_are_kept_exactly_and_discovered_ones_follow_their_discovery() {
        let Some((_, states)) = crate::test_support::replicas(2).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "cloud-jobs").await;
        let (static_acct, discovered_acct) = ("6f1c2b0a-3d4e-4f5a-8b9c-0d1e2f3a4b5c", "0a9b8c7d-6e5f-4a3b-9c2d-1e0f9a8b7c6d");
        planned(st, SCOPES, &[d(&org, static_acct, "eu-west-1"), d(&org, static_acct, "us-east-1"), d(&org, discovered_acct, "")]).await;
        assert_eq!(scopes(st, &org).await, ["", "eu-west-1", "us-east-1"]);

        // The discovery finds two subscriptions, then one.
        let discovery = JobKey { provider: SCOPES.into(), org_id: org.clone(), account_id: discovered_acct.into(), scope: "".into(), collected_until: None };
        record(st, &discovery, &Outcome::Discovered(vec!["sub-a".into(), "sub-b".into()]), 600).await.unwrap();
        assert_eq!(scopes(st, &org).await, ["", "eu-west-1", "sub-a", "sub-b", "us-east-1"]);
        record(st, &discovery, &Outcome::Discovered(vec!["sub-b".into()]), 600).await.unwrap();
        assert_eq!(scopes(st, &org).await, ["", "eu-west-1", "sub-b", "us-east-1"]);

        // Planning keeps discovered scopes, drops a static scope no longer listed, then a removed account's jobs.
        planned(st, SCOPES, &[d(&org, static_acct, "eu-west-1"), d(&org, discovered_acct, "")]).await;
        assert_eq!(scopes(st, &org).await, ["", "eu-west-1", "sub-b"]);
        planned(st, SCOPES, &[d(&org, static_acct, "eu-west-1")]).await;
        assert_eq!(scopes(st, &org).await, ["eu-west-1"]);

        // Two servers claim at once: the job goes to one of them.
        planned(st, SCOPES, &[d(&org, static_acct, "eu-west-1"), d(&org, static_acct, "eu-central-1")]).await;
        let (a, b) = tokio::join!(claim(&states[0], SCOPES, 1000), claim(&states[1], SCOPES, 1000));
        let mine: Vec<JobKey> = a.unwrap().into_iter().chain(b.unwrap()).filter(|j| j.org_id == org).collect();
        assert_eq!(mine.len(), 2);
        assert!(claim(st, SCOPES, 1000).await.unwrap().iter().all(|j| j.org_id != org));
    }

    #[tokio::test]
    async fn the_watermark_moves_only_on_success_and_failures_are_the_status() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "cloud-record").await;
        let acct = "6f1c2b0a-3d4e-4f5a-8b9c-0d1e2f3a4b5c";
        planned(st, RECORD, &[d(&org, acct, "eu-west-1")]).await;
        let job = JobKey { provider: RECORD.into(), org_id: org.clone(), account_id: acct.into(), scope: "eu-west-1".into(), collected_until: None };
        let issue = Issue { kind: "access_denied", message: "denied".into(), missing_action: Some("cloudwatch:ListMetrics".into()), backoff: Backoff::Interval };
        record(st, &job, &Outcome::Failed(issue), 600).await.unwrap();
        let s = &status(st, RECORD, &org, acct).await.unwrap()[0];
        assert_eq!((s.collected_until, s.error.as_ref().map(|e| e.missing_action.clone())), (None, Some(Some("cloudwatch:ListMetrics".into()))));
        record(st, &job, &Outcome::Collected { end_ms: 1_700_000_000_000, metrics: 3, points: 90, warning: None }, 600).await.unwrap();
        let s = &status(st, RECORD, &org, acct).await.unwrap()[0];
        assert_eq!((s.collected_until, s.metrics, s.points, s.error.is_none()), (Some(1_700_000_000_000), 3, 90, true));
    }
}
