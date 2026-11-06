//! The GitLab CI poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider `gitlab`), so
//! several servers share the projects (`FOR UPDATE SKIP LOCKED`).
//!
//! - **Discovery** (scope `""`, hourly and right after the connection is saved): the group's projects
//!   (with its subgroups when asked) and the listed ones become one job each (scope `<id>:<path>`).
//! - **A project's run** ([`sync_project`], every minute) lists the pipelines updated since its cursor
//!   (`collected_until`; the first sync looks back [`FIRST_SYNC_LOOKBACK_MS`]), oldest update first, and
//!   sends each finished one not yet exported as a CI/CD trace (OTLP JSON on `oc.otlp-json.traces`, with
//!   the org's headers) once Kafka acknowledged it, then records it ([`Ledger`]). The cursor moves to
//!   the last pipeline handled, so a run cut short (request budget, throttling) resumes there.
//! - **Limits**: at most [`MAX_REQUESTS_PER_RUN`] GitLab requests per project run; HTTP 429 backs the
//!   job off exponentially; at most [`MAX_PROJECTS`] projects per connection.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_json::{json, Value};

use super::api::{self, GitlabApi, GitlabError, PER_PAGE};
use super::otlp;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::routes::portal_sources::parse_time;
use crate::state::AppState;

pub const PROVIDER: &str = "gitlab";
/// The Kafka topic the collector reads OTLP JSON traces from (its CI routing sends them to `otel_ci_traces`).
pub const TRACES_TOPIC: &str = "oc.otlp-json.traces";
/// How often a project is polled.
pub const INTERVAL_SECS: i64 = 60;
/// How far back the first sync of a project goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 7 * 86_400_000;
/// Each run lists again from a little before the cursor (pipelines updated in the same second).
pub const CURSOR_OVERLAP_MS: i64 = 60_000;
/// GitLab requests per project run.
pub const MAX_REQUESTS_PER_RUN: usize = 100;
/// Projects followed per connection.
pub const MAX_PROJECTS: usize = 500;
/// With `all_projects`, the projects active in this many days (the hourly discovery adds those that wake up).
pub const ACTIVE_PROJECTS_DAYS: i64 = 30;
/// Pages of deployments read per run (environments of deploy jobs).
const MAX_DEPLOYMENT_PAGES: usize = 3;
/// How long exported pipelines are remembered (beyond the CI table's 30 days).
pub const LEDGER_DAYS: i64 = 35;

/// An org's connection, with its token unsealed.
#[derive(Clone, Debug)]
pub struct Connection {
    pub base_url: String,
    pub token: String,
    pub group_path: String,
    pub include_subgroups: bool,
    pub project_paths: Vec<String>,
    /// Every project the token sees (active lately), instead of or besides a group and listed projects.
    pub all_projects: bool,
}

/// Which pipelines were exported already.
#[async_trait::async_trait]
pub trait Ledger: Send + Sync {
    /// The ones among `pipelines` (of a project) already exported.
    async fn exported(&self, org: &str, project_id: u64, pipelines: &[u64]) -> Result<HashSet<u64>, String>;
    async fn mark(&self, org: &str, project_id: u64, pipeline_id: u64, jobs: usize) -> Result<(), String>;
}

/// The ledger in Postgres (`gitlab_ci_exported`).
pub struct PgLedger(pub sqlx::PgPool);

#[async_trait::async_trait]
impl Ledger for PgLedger {
    async fn exported(&self, org: &str, project_id: u64, pipelines: &[u64]) -> Result<HashSet<u64>, String> {
        let ids: Vec<i64> = pipelines.iter().map(|p| *p as i64).collect();
        let rows: Vec<i64> = sqlx::query_scalar("SELECT pipeline_id FROM gitlab_ci_exported WHERE org_id = $1 AND project_id = $2 AND pipeline_id = ANY($3)")
            .bind(org)
            .bind(project_id as i64)
            .bind(&ids)
            .fetch_all(&self.0)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|p| p as u64).collect())
    }

    async fn mark(&self, org: &str, project_id: u64, pipeline_id: u64, jobs: usize) -> Result<(), String> {
        sqlx::query("INSERT INTO gitlab_ci_exported (org_id, project_id, pipeline_id, jobs) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
            .bind(org)
            .bind(project_id as i64)
            .bind(pipeline_id as i64)
            .bind(jobs as i32)
            .execute(&self.0)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// A project's job scope: `<id>:<path>` (the path is for people reading the status).
pub fn scope(id: u64, path: &str) -> String {
    format!("{id}:{path}")
}

/// The project id of a scope.
pub fn scope_project_id(scope: &str) -> Option<u64> {
    scope.split_once(':').map_or(scope, |(id, _)| id).parse().ok()
}

/// Where a project's run lists from: a little before the cursor, never further back than the first
/// sync's lookback.
pub fn updated_after(cursor: Option<i64>, now_ms: i64) -> i64 {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    cursor.map_or(floor, |c| (c - CURSOR_OVERLAP_MS).max(floor))
}

/// A GitLab error as the connection's issue.
pub fn issue(e: &GitlabError) -> Issue {
    let (kind, backoff) = match e {
        GitlabError::Throttled(_) => ("throttled", Backoff::Exponential),
        GitlabError::Auth(_) => ("auth", Backoff::Interval),
        GitlabError::NotFound(_) => ("access_denied", Backoff::Interval),
        GitlabError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

/// GitLab requests with a budget: past it, [`Budget::get`] refuses.
struct Budget<'a> {
    api: &'a dyn GitlabApi,
    conn: &'a Connection,
    left: usize,
}

impl Budget<'_> {
    async fn get(&mut self, path: &str) -> Result<Value, GitlabError> {
        if self.left == 0 {
            return Err(GitlabError::Other("request budget spent".into()));
        }
        self.left -= 1;
        self.api.get(&self.conn.base_url, &self.conn.token, path).await
    }

    async fn list(&mut self, path: &str) -> Result<Vec<Value>, GitlabError> {
        match self.get(path).await? {
            Value::Array(items) => Ok(items),
            _ => Err(GitlabError::Other("GitLab answered with something other than a list".into())),
        }
    }
}

/// The projects a connection follows: `(id, path_with_namespace)`, by id.
pub async fn discover(api: &dyn GitlabApi, conn: &Connection, now_ms: i64) -> Result<Vec<(u64, String)>, GitlabError> {
    let mut found: BTreeMap<u64, String> = BTreeMap::new();
    let mut add = |p: &Value| {
        if let (Some(id), Some(path)) = (p["id"].as_u64(), p["path_with_namespace"].as_str()) {
            found.insert(id, path.to_string());
        }
    };
    if !conn.group_path.is_empty() {
        for page in 1.. {
            let items = match api.get(&conn.base_url, &conn.token, &api::group_projects_path(&conn.group_path, conn.include_subgroups, page)).await? {
                Value::Array(items) => items,
                _ => return Err(GitlabError::Other("GitLab answered with something other than a list of projects".into())),
            };
            items.iter().for_each(&mut add);
            if items.len() < PER_PAGE || page * PER_PAGE >= MAX_PROJECTS {
                break;
            }
        }
    }
    if conn.all_projects {
        let since = now_ms - ACTIVE_PROJECTS_DAYS * 86_400_000;
        for page in 1.. {
            let items = match api.get(&conn.base_url, &conn.token, &api::visible_projects_path(since, page)).await? {
                Value::Array(items) => items,
                _ => return Err(GitlabError::Other("GitLab answered with something other than a list of projects".into())),
            };
            items.iter().for_each(&mut add);
            if items.len() < PER_PAGE || page * PER_PAGE >= MAX_PROJECTS {
                break;
            }
        }
    }
    for path in &conn.project_paths {
        add(&api.get(&conn.base_url, &conn.token, &api::project_path(path)).await?);
    }
    Ok(found.into_iter().take(MAX_PROJECTS).collect())
}

/// Job id → environment, from the project's deployments updated in the window.
async fn environments(b: &mut Budget<'_>, project_id: u64, after_ms: i64) -> Result<HashMap<u64, String>, GitlabError> {
    let mut out = HashMap::new();
    for page in 1..=MAX_DEPLOYMENT_PAGES {
        let items = b.list(&api::deployments_path(project_id, after_ms, page)).await?;
        for d in &items {
            if let (Some(job), Some(env)) = (d.pointer("/deployable/id").and_then(Value::as_u64), d.pointer("/environment/name").and_then(Value::as_str)) {
                out.insert(job, env.to_string());
            }
        }
        if items.len() < PER_PAGE {
            break;
        }
    }
    Ok(out)
}

/// One pipeline's trace: its detail, jobs and commit.
async fn trace(b: &mut Budget<'_>, project: &Value, project_id: u64, pipeline_id: u64, envs: &HashMap<u64, String>) -> Result<(Value, usize), GitlabError> {
    let pipeline = b.get(&api::pipeline_path(project_id, pipeline_id)).await?;
    let mut jobs = Vec::new();
    for page in 1.. {
        let items = b.list(&api::jobs_path(project_id, pipeline_id, page)).await?;
        let n = items.len();
        jobs.extend(items);
        if n < PER_PAGE {
            break;
        }
    }
    let commit = match jobs.iter().find_map(|j| j.get("commit").filter(|c| c.is_object())) {
        Some(c) => c.clone(),
        None => match api::commit_path(project_id, pipeline["sha"].as_str().unwrap_or_default()) {
            Some(path) => b.get(&path).await.unwrap_or(Value::Null),
            None => Value::Null,
        },
    };
    let ran = jobs.iter().filter(|j| j["started_at"].is_string()).count();
    Ok((otlp::pipeline_trace(project, &pipeline, &jobs, &commit, envs), ran))
}

/// Requests a pipeline's export takes at most (detail, one page of jobs, the commit).
const REQUESTS_PER_PIPELINE: usize = 3;

/// Polls one project: see the module documentation. `cursor`: the project's `collected_until`.
#[allow(clippy::too_many_arguments)]
pub async fn sync_project(
    api: &dyn GitlabApi, sink: &dyn MetricSink, ledger: &dyn Ledger, headers: &[(String, String)], org: &str, conn: &Connection, project_id: u64, cursor: Option<i64>, now_ms: i64,
) -> Outcome {
    let after = updated_after(cursor, now_ms);
    let mut b = Budget { api, conn, left: MAX_REQUESTS_PER_RUN };
    let mut next_cursor = cursor.unwrap_or(after);
    let (mut pipelines, mut jobs) = (0usize, 0usize);
    let failed = |e: GitlabError, next_cursor: i64, pipelines: usize, jobs: usize| {
        if pipelines == 0 {
            Outcome::Failed(issue(&e))
        } else {
            Outcome::Collected { end_ms: next_cursor, metrics: pipelines, points: jobs, warning: Some(issue(&e)) }
        }
    };
    let project = match b.get(&api::project_path(&project_id.to_string())).await {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    // Deployments are best effort (a project may have them off): only throttling stops the run.
    let envs = match environments(&mut b, project_id, after).await {
        Ok(envs) => envs,
        Err(e @ GitlabError::Throttled(_)) => return Outcome::Failed(issue(&e)),
        Err(_) => HashMap::new(),
    };
    'pages: for page in 1.. {
        let listed = match b.list(&api::pipelines_path(project_id, after, page)).await {
            Ok(l) => l,
            Err(e) => return failed(e, next_cursor, pipelines, jobs),
        };
        let ids: Vec<u64> = listed.iter().filter_map(|p| p["id"].as_u64()).collect();
        let done = match ledger.exported(org, project_id, &ids).await {
            Ok(d) => d,
            Err(e) => return failed(GitlabError::Other(format!("ledger: {e}")), next_cursor, pipelines, jobs),
        };
        for p in &listed {
            let Some(id) = p["id"].as_u64() else { continue };
            let updated = parse_time(p["updated_at"].as_str().unwrap_or_default());
            if otlp::is_finished(p["status"].as_str().unwrap_or_default()) && !done.contains(&id) {
                if b.left < REQUESTS_PER_PIPELINE {
                    break 'pages;
                }
                let (trace, ran) = match trace(&mut b, &project, project_id, id, &envs).await {
                    Ok(t) => t,
                    Err(e) => return failed(e, next_cursor, pipelines, jobs),
                };
                let body = serde_json::to_vec(&json!({ "resourceSpans": [trace] })).unwrap_or_default();
                if let Err(e) = sink.publish(org, headers, body).await {
                    return failed(GitlabError::Other(e), next_cursor, pipelines, jobs);
                }
                if let Err(e) = ledger.mark(org, project_id, id, ran).await {
                    return failed(GitlabError::Other(format!("ledger: {e}")), next_cursor, pipelines, jobs);
                }
                pipelines += 1;
                jobs += ran;
            }
            next_cursor = next_cursor.max(updated);
        }
        if listed.len() < PER_PAGE {
            break;
        }
    }
    Outcome::Collected { end_ms: next_cursor, metrics: pipelines, points: jobs, warning: None }
}

/// The poller: GitLab through `api`, traces to `sink`.
pub struct GitlabCrawler {
    pub api: Arc<dyn GitlabApi>,
    pub sink: Arc<dyn MetricSink>,
}

/// An org's connection with its token unsealed, and its id.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<(String, Connection)>, sqlx::Error> {
    /// id, base_url, token_enc, group_path, include_subgroups, project_paths, all_projects
    type Row = (String, String, String, String, bool, Vec<String>, bool);
    let row: Option<Row> =
        sqlx::query_as("SELECT id::text, base_url, token_enc, group_path, include_subgroups, project_paths, all_projects FROM gitlab_ci_connections WHERE org_id = $1")
            .bind(org)
            .fetch_optional(&st.postgres)
            .await?;
    Ok(row.and_then(|(id, base_url, sealed, group_path, include_subgroups, project_paths, all_projects)| {
        let token = st.secrets.decrypt(&sealed).ok()?;
        Some((id, Connection { base_url, token, group_path, include_subgroups, project_paths, all_projects }))
    }))
}

#[async_trait::async_trait]
impl Crawler for GitlabCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One discovery job per connection.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.org_id, c.id::text FROM gitlab_ci_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, String::new())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, conn)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if id != job.account_id {
            return Ok(Outcome::Idle);
        }
        if job.scope.is_empty() {
            sqlx::query("DELETE FROM gitlab_ci_exported WHERE org_id = $1 AND exported_at < now() - make_interval(days => $2)").bind(&job.org_id).bind(LEDGER_DAYS as i32).execute(&st.postgres).await?;
            return Ok(match discover(self.api.as_ref(), &conn, now_ms).await {
                Ok(projects) => Outcome::Discovered(projects.iter().map(|(id, path)| scope(*id, path)).collect()),
                Err(e) => Outcome::Failed(issue(&e)),
            });
        }
        let Some(project_id) = scope_project_id(&job.scope) else { return Ok(Outcome::Idle) };
        let headers = jobs::org_headers(st, &job.org_id).await?;
        let ledger = PgLedger(st.postgres.clone());
        Ok(sync_project(self.api.as_ref(), self.sink.as_ref(), &ledger, &headers, &job.org_id, &conn, project_id, job.collected_until, now_ms).await)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::super::fixtures::{self, FakeGitlab};
    use super::*;
    use crate::cloud::testing::FakeSink;

    #[derive(Default)]
    struct FakeLedger(StdMutex<HashSet<(u64, u64)>>);

    #[async_trait::async_trait]
    impl Ledger for FakeLedger {
        async fn exported(&self, _org: &str, project_id: u64, pipelines: &[u64]) -> Result<HashSet<u64>, String> {
            let done = self.0.lock().unwrap();
            Ok(pipelines.iter().copied().filter(|p| done.contains(&(project_id, *p))).collect())
        }
        async fn mark(&self, _org: &str, project_id: u64, pipeline_id: u64, _jobs: usize) -> Result<(), String> {
            self.0.lock().unwrap().insert((project_id, pipeline_id));
            Ok(())
        }
    }

    fn conn() -> Connection {
        Connection { base_url: "https://gitlab.example.com".into(), token: "glpat-test".into(), group_path: String::new(), include_subgroups: true, project_paths: vec![], all_projects: false }
    }

    const NOW: i64 = 1_793_710_000_000;
    const P: u64 = 4211;

    /// A project with `n` pipelines (ids from 1000) listed by the first sync, the last `running` ones still running.
    fn gitlab(n: u64, running: u64) -> FakeGitlab {
        let g = FakeGitlab::default();
        let after = updated_after(None, NOW);
        g.answer(api::project_path("4211"), fixtures::project());
        g.answer(api::deployments_path(P, after, 1), fixtures::deployments());
        let listed: Vec<Value> = (0..n)
            .map(|i| {
                let status = if i >= n - running { "running" } else { "success" };
                fixtures::pipeline(1000 + i, status, &api::iso(1_793_700_000_000 + i as i64 * 1000).replace('Z', ".000Z"))
            })
            .collect();
        for (page, chunk) in listed.chunks(PER_PAGE).enumerate() {
            g.answer(api::pipelines_path(P, after, page + 1), json!(chunk));
        }
        if listed.len().is_multiple_of(PER_PAGE) {
            g.answer(api::pipelines_path(P, after, listed.len() / PER_PAGE + 1), json!([]));
        }
        for p in &listed {
            let id = p["id"].as_u64().unwrap();
            g.answer(api::pipeline_path(P, id), p.clone());
            g.answer(api::jobs_path(P, id, 1), fixtures::jobs());
        }
        g
    }

    fn headers() -> Vec<(String, String)> {
        vec![("x-oc-org".into(), "acme".into()), ("x-oc-retention-days".into(), "15".into())]
    }

    #[test]
    fn the_first_sync_is_bounded_and_later_ones_overlap_the_cursor() {
        assert_eq!(updated_after(None, NOW), NOW - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(updated_after(Some(NOW - 300_000), NOW), NOW - 300_000 - CURSOR_OVERLAP_MS);
        // A cursor older than the lookback (the server was off for weeks) is bounded too.
        assert_eq!(updated_after(Some(NOW - 30 * 86_400_000), NOW), NOW - FIRST_SYNC_LOOKBACK_MS);
    }

    #[test]
    fn scopes_carry_the_project_id() {
        assert_eq!(scope(4211, "example-org/storefront-api"), "4211:example-org/storefront-api");
        assert_eq!(scope_project_id("4211:example-org/storefront-api"), Some(4211));
        assert_eq!(scope_project_id("4211"), Some(4211));
        assert_eq!(scope_project_id("x:y"), None);
    }

    #[tokio::test]
    async fn finished_pipelines_are_exported_once_with_the_orgs_headers() {
        let g = gitlab(3, 1);
        let (sink, ledger) = (FakeSink::default(), FakeLedger::default());
        let out = sync_project(&g, &sink, &ledger, &headers(), "acme", &conn(), P, None, NOW).await;
        // Two finished pipelines, three jobs that ran each; the cursor is the last listed update (the running one).
        assert_eq!(out, Outcome::Collected { end_ms: 1_793_700_002_000, metrics: 2, points: 6, warning: None });
        let records = sink.records.lock().unwrap().clone();
        assert_eq!(records.len(), 2);
        let (org, hdrs, body) = &records[0];
        assert_eq!(org, "acme");
        assert_eq!(hdrs, &headers());
        let spans = &body["resourceSpans"][0]["scopeSpans"][0]["spans"];
        assert_eq!(spans[0]["traceId"], otlp::trace_id(P, 1000));
        let deploy = spans.as_array().unwrap().iter().find(|s| s["name"] == "deploy-production").unwrap();
        assert!(deploy["attributes"].as_array().unwrap().iter().any(|a| a["key"] == "deployment.environment.name" && a["value"]["stringValue"] == "production"));

        // The next run lists from the cursor: nothing new is sent again.
        let after = updated_after(Some(1_793_700_002_000), NOW);
        g.answer(api::deployments_path(P, after, 1), json!([]));
        let mut listed: Vec<Value> = (0..3).map(|i| fixtures::pipeline(1000 + i, "success", "2026-11-03T10:00:02.000Z")).collect();
        listed[2]["updated_at"] = json!("2026-11-03T10:09:00.000Z");
        g.answer(api::pipelines_path(P, after, 1), json!(listed));
        g.answer(api::pipeline_path(P, 1002), listed[2].clone());
        let out = sync_project(&g, &sink, &ledger, &headers(), "acme", &conn(), P, Some(1_793_700_002_000), NOW).await;
        // Only the pipeline that finished since.
        assert_eq!(out, Outcome::Collected { end_ms: parse_time("2026-11-03T10:09:00.000Z"), metrics: 1, points: 3, warning: None });
        assert_eq!(sink.records.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_run_stops_at_its_request_budget_and_the_next_one_resumes() {
        let g = gitlab(150, 0);
        let (sink, ledger) = (FakeSink::default(), FakeLedger::default());
        let Outcome::Collected { end_ms, metrics, .. } = sync_project(&g, &sink, &ledger, &headers(), "acme", &conn(), P, None, NOW).await else { panic!() };
        assert!(g.count() <= MAX_REQUESTS_PER_RUN, "{}", g.count());
        assert!(metrics > 0 && metrics < 150);
        // The cursor is the last exported pipeline's update.
        assert_eq!(end_ms, 1_793_700_000_000 + (metrics as i64 - 1) * 1000);
        assert_eq!(ledger.0.lock().unwrap().len(), metrics);
    }

    #[tokio::test]
    async fn throttling_keeps_the_progress_and_backs_off() {
        // Throttled from the start: a failure, backing off exponentially.
        let g = FakeGitlab { throttle_after: Some(0), ..Default::default() };
        let out = sync_project(&g, &FakeSink::default(), &FakeLedger::default(), &headers(), "acme", &conn(), P, None, NOW).await;
        let Outcome::Failed(i) = out else { panic!("{out:?}") };
        assert_eq!((i.kind, i.backoff), ("throttled", Backoff::Exponential));

        // Throttled after a few pipelines: what was sent counts, the cursor stays on it.
        let mut g = gitlab(10, 0);
        g.throttle_after = Some(3 + 2 * 3);
        let sink = FakeSink::default();
        let out = sync_project(&g, &sink, &FakeLedger::default(), &headers(), "acme", &conn(), P, None, NOW).await;
        let Outcome::Collected { end_ms, metrics: 3, warning: Some(w), .. } = out else { panic!("{out:?}") };
        assert_eq!(w.kind, "throttled");
        assert_eq!(end_ms, 1_793_700_002_000);
        assert_eq!(sink.records.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn deployments_are_best_effort() {
        let g = gitlab(2, 0);
        g.answers.lock().unwrap().remove(&api::deployments_path(P, updated_after(None, NOW), 1));
        let out = sync_project(&g, &FakeSink::default(), &FakeLedger::default(), &headers(), "acme", &conn(), P, None, NOW).await;
        assert!(matches!(out, Outcome::Collected { metrics: 2, warning: None, .. }), "{out:?}");
    }

    #[tokio::test]
    async fn nothing_is_recorded_when_kafka_refuses() {
        let g = gitlab(2, 0);
        let ledger = FakeLedger::default();
        let sink = FakeSink { fail: true, ..Default::default() };
        let out = sync_project(&g, &sink, &ledger, &headers(), "acme", &conn(), P, None, NOW).await;
        assert!(matches!(out, Outcome::Failed(_)), "{out:?}");
        assert!(ledger.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn discovery_reads_the_group_pages_and_the_listed_projects() {
        let g = FakeGitlab::default();
        let page1: Vec<Value> = (0..PER_PAGE as u64).map(|i| json!({"id": 10 + i, "path_with_namespace": format!("example-org/svc-{i}")})).collect();
        g.answer(api::group_projects_path("example-org", true, 1), json!(page1));
        g.answer(api::group_projects_path("example-org", true, 2), json!([{"id": 4211, "path_with_namespace": "example-org/storefront-api"}]));
        g.answer(api::project_path("example-org/storefront-api"), fixtures::project());
        g.answer(api::project_path("other-group/billing"), json!({"id": 7, "path_with_namespace": "other-group/billing"}));
        let c = Connection { group_path: "example-org".into(), project_paths: vec!["example-org/storefront-api".into(), "other-group/billing".into()], ..conn() };
        let found = discover(&g, &c, NOW).await.unwrap();
        assert_eq!(found.len(), PER_PAGE + 2);
        assert_eq!(found[0], (7, "other-group/billing".to_string()));
        assert!(found.contains(&(4211, "example-org/storefront-api".to_string())));

        let missing = Connection { project_paths: vec!["example-org/gone".into()], ..conn() };
        assert!(matches!(discover(&g, &missing, NOW).await, Err(GitlabError::NotFound(_))));
    }

    #[tokio::test]
    async fn all_projects_are_those_the_token_sees_active_lately() {
        let g = FakeGitlab::default();
        let since = NOW - ACTIVE_PROJECTS_DAYS * 86_400_000;
        let page1: Vec<Value> = (0..PER_PAGE as u64).map(|i| json!({"id": 100 + i, "path_with_namespace": format!("group-{}/svc-{i}", i % 7)})).collect();
        g.answer(api::visible_projects_path(since, 1), json!(page1));
        g.answer(api::visible_projects_path(since, 2), json!([{"id": 4211, "path_with_namespace": "example-org/storefront-api"}]));
        let c = Connection { all_projects: true, project_paths: vec![], ..conn() };
        let found = discover(&g, &c, NOW).await.unwrap();
        assert_eq!(found.len(), PER_PAGE + 1);
        assert!(found.contains(&(4211, "example-org/storefront-api".to_string())));
        assert!(api::visible_projects_path(since, 1).contains("archived=false"));
    }
}
