//! Packages' streams on the cloud crawlers' jobs (`cloud_crawl_jobs`: provider = connector id, account = connection
//! id, scope = stream id), so several servers share them (`FOR UPDATE SKIP LOCKED`). One loop serves every package:
//! it plans each package's jobs every minute (one per enabled stream of every enabled connection), claims due jobs of
//! any of them and runs them ([`run_job`]):
//!
//! 1. the connection is read and unsealed, its stream's state loaded, its request quotas counted;
//! 2. the engine runs the stream (`oc_connectors::engine::run`);
//! 3. what it read is published to Kafka with the org's headers (logs on `oc.logs.json`, metrics on
//!    `oc.otlp-json.metrics`, events on `oc.otlp-json.logs`) and costs written to `otel.cloud_costs`, each
//!    acknowledged;
//! 4. only then is the state saved (a failed publish reads the same records again), the quotas counted and rotated
//!    secrets saved.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oc_connectors::engine::{self, Issue, Runtime};
use oc_connectors::manifest::IssueKind;
use oc_connectors::output;

use super::{registry, store, PackageConnector};
use crate::cloud::jobs::{self, Backoff, JobKey, MetricSink, Outcome};
use crate::state::AppState;

pub const LOGS_TOPIC: &str = "oc.logs.json";
pub const METRICS_TOPIC: &str = "oc.otlp-json.metrics";
pub const EVENTS_TOPIC: &str = "oc.otlp-json.logs";
/// Logs per Kafka record (well under the broker's 1 MiB message limit).
pub const LOGS_PER_RECORD: usize = 200;
pub const POINTS_PER_RECORD: usize = 2_000;
pub const EVENTS_PER_RECORD: usize = 500;

pub use oc_connectors::catalog::interval_secs;

/// Where runs publish.
pub struct Sinks {
    pub logs: Arc<dyn MetricSink>,
    pub metrics: Arc<dyn MetricSink>,
    pub events: Arc<dyn MetricSink>,
}

pub struct Runner {
    pub rt: Arc<Runtime>,
    pub sinks: Sinks,
}

/// An engine issue as the job's.
pub fn issue(i: &Issue) -> jobs::Issue {
    let (kind, backoff) = match i.kind {
        IssueKind::Auth => ("auth", Backoff::Interval),
        IssueKind::AccessDenied => ("access_denied", Backoff::Interval),
        IssueKind::Throttled => ("throttled", Backoff::Exponential),
        IssueKind::Disabled => ("disabled", Backoff::Day),
        IssueKind::Other => ("other", Backoff::Exponential),
    };
    jobs::Issue { kind, message: i.message.clone(), missing_action: None, backoff }
}

fn other(message: String) -> Outcome {
    Outcome::Failed(jobs::Issue { kind: "other", message, missing_action: None, backoff: Backoff::Exponential })
}

/// The jobs a package's connections call for: `(org, connection, stream)`.
pub async fn desired(st: &AppState, connector: &str) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.org_id, c.id::text, s FROM connector_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active', unnest(c.streams) AS s \
         WHERE c.connector_id = $1 AND c.enabled",
    )
    .bind(connector)
    .fetch_all(&st.postgres)
    .await
}

async fn publish(sink: &dyn MetricSink, org: &str, headers: &[(String, String)], bodies: Vec<Vec<u8>>) -> Result<(), String> {
    for body in bodies {
        sink.publish(org, headers, body).await.map_err(|e| format!("Kafka: {e}"))?;
    }
    Ok(())
}

/// Runs one job: see the module documentation.
pub async fn run_job(st: &AppState, runner: &Runner, pkg: &PackageConnector, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
    let m = &pkg.package.manifest;
    let Some(stream) = m.streams.iter().find(|s| s.id == job.scope) else { return Ok(Outcome::Idle) };
    let Some(saved) = store::get(st, &job.org_id, &m.id, &job.account_id).await? else { return Ok(Outcome::Idle) };
    if !saved.enabled || !saved.streams.contains(&stream.id) {
        return Ok(Outcome::Idle);
    }
    let mut conn = match store::unseal(st, &saved) {
        Ok(c) => c,
        Err(e) => return Ok(Outcome::Failed(jobs::Issue { kind: "auth", message: e, missing_action: None, backoff: Backoff::Interval })),
    };
    conn.config = engine::effective_config(m, &conn.config);
    let state = store::state(st, &job.org_id, &saved.id, &stream.id).await?;
    let mut left: Option<u32> = None;
    for (scope, budget) in [("", &m.budget), (stream.id.as_str(), &stream.budget)] {
        if let Some(b) = budget {
            let l = store::budget_left(st, &job.org_id, &saved.id, scope, b).await?;
            left = Some(left.map_or(l, |x| x.min(l)));
        }
    }
    let run = match engine::run(m, stream, &conn, state, now_ms, &runner.rt, left).await {
        Ok(r) => r,
        Err(i) => return Ok(Outcome::Failed(issue(&i))),
    };
    let headers = jobs::org_headers(st, &job.org_id).await?;
    let mapped = &run.mapped;
    let logs: Vec<Vec<u8>> = mapped.logs.chunks(LOGS_PER_RECORD).map(|c| serde_json::to_vec(c).unwrap_or_default()).collect();
    let points: Vec<Vec<u8>> = mapped.points.chunks(POINTS_PER_RECORD).map(|c| output::metrics_otlp(&m.id, c)).collect();
    let events: Vec<Vec<u8>> = mapped.events.chunks(EVENTS_PER_RECORD).map(output::events_otlp).collect();
    for (sink, bodies) in [(&runner.sinks.logs, logs), (&runner.sinks.metrics, points), (&runner.sinks.events, events)] {
        if let Err(e) = publish(sink.as_ref(), &job.org_id, &headers, bodies).await {
            return Ok(other(e));
        }
    }
    if !mapped.costs.is_empty() {
        let lines = cost_lines(&job.org_id, &saved.id, &stream.id, &mapped.costs, now_ms);
        if let Err(e) = crate::routes::cloud_cost::insert(st, &lines).await {
            return Ok(other(format!("writing costs failed: {e}")));
        }
    }
    store::save_state(st, &job.org_id, &saved.id, &stream.id, &run.state).await?;
    for (scope, budget) in [("", &m.budget), (stream.id.as_str(), &stream.budget)] {
        if let Some(b) = budget {
            store::budget_use(st, &job.org_id, &saved.id, scope, b, run.requests).await?;
        }
    }
    for (field, value) in &run.rotated {
        store::rotate(st, &job.org_id, &saved.id, field, value).await?;
        runner.rt.forget(&saved.id);
    }
    let points = mapped.logs.len() + mapped.points.len() + mapped.events.len() + mapped.costs.len();
    let minute = now_ms - now_ms.rem_euclid(60_000);
    Ok(Outcome::Collected { end_ms: run.state.collected_until().unwrap_or(minute), metrics: run.records, points, warning: run.warning.as_ref().map(issue) })
}

/// A run's costs as `otel.cloud_costs` rows: one file per connection and stream, so a day read again replaces its rows.
pub fn cost_lines(org: &str, connection: &str, stream: &str, costs: &[output::Cost], now_ms: i64) -> Vec<crate::routes::cloud_cost::CostLine> {
    costs
        .iter()
        .map(|c| crate::routes::cloud_cost::CostLine {
            org: org.to_string(),
            file_id: format!("connector:{connection}:{stream}"),
            line_no: c.line,
            date: c.date.clone(),
            provider: c.provider.clone(),
            service: c.service.clone(),
            sub_account: c.account.clone(),
            region: String::new(),
            zone: String::new(),
            category: "usage".into(),
            description: c.usage_type.clone(),
            resource_id: String::new(),
            pricing_unit: String::new(),
            quantity: c.usage_amount,
            billed: c.cost,
            effective: c.cost,
            list: c.cost,
            pricing_category: "Standard".into(),
            commitment_id: String::new(),
            commitment_type: String::new(),
            commitment_status: String::new(),
            currency: c.currency.clone(),
            tags: c.tags.clone(),
            source: "connector".into(),
            version: now_ms as u64,
            deleted: 0,
        })
        .collect()
}

/// The loop: plans every package's jobs every minute, claims up to `concurrency` due jobs of any of them and runs them.
pub async fn run(st: AppState, runner: Arc<Runner>, concurrency: i64) {
    let packages = &registry().packages;
    let providers: Vec<String> = packages.iter().filter(|p| !p.package.manifest.streams.is_empty() && p.package.manifest.native.is_none()).map(|p| p.package.manifest.id.clone()).collect();
    if providers.is_empty() {
        return;
    }
    let mut planned_at: Option<Instant> = None;
    loop {
        if planned_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            for id in &providers {
                let planned = match desired(&st, id).await {
                    Ok(d) => jobs::plan(&st, id, &d).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = planned {
                    tracing::warn!(connector = %id, error = %e, "connectors: planning failed");
                }
            }
            planned_at = Some(Instant::now());
        }
        let due = jobs::claim_any(&st, &providers, concurrency).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "connectors: claiming jobs failed");
            vec![]
        });
        if due.is_empty() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let runs = due.iter().map(|job| {
            let runner = runner.clone();
            let st = st.clone();
            async move {
                let Some(pkg) = registry().package(&job.provider) else { return Ok::<_, sqlx::Error>(Outcome::Idle) };
                let interval = pkg.package.manifest.streams.iter().find(|s| s.id == job.scope).map_or(60, interval_secs);
                let outcome = run_job(&st, &runner, pkg, job, now_ms).await?;
                jobs::record(&st, job, &outcome, interval).await?;
                Ok(outcome)
            }
        });
        for (job, result) in due.iter().zip(futures::future::join_all(runs).await) {
            match result {
                Ok(Outcome::Failed(issue)) => tracing::info!(connector = %job.provider, org = %job.org_id, stream = %job.scope, error = %issue.message, "connector run failed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(connector = %job.provider, error = %e, "connectors: a run failed"),
            }
        }
    }
}
