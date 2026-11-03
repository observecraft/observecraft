//! Assist's background runs (investigations, security verdicts, agents, coding sessions) and Assist
//! Investigation: monitors investigated automatically, memories, the knowledge base, rate limits and
//! reports. Runs are queued in Postgres and executed by any server (`run_worker`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::assist::agent::{self, Loop, Outcome};
use crate::assist::llm::{Msg, Provider, Usage};
use crate::assist::prompt::{run_prompt, RunPrompt};
use crate::assist::runs::{CodeChange, Conclusion, Kind, RunTools};
use crate::assist::tools::OrgTools;
use crate::auth::{Access, Admin, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::routes::logs::TimeRange;
use crate::routes::{assist_provider, monitors, notifications};
use crate::state::{AppState, OrgId};

// ---------------------------------------------------------------- steps

/// What a run did, as it goes: its text and the tools it called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunStep {
    Text { text: String },
    Tool { id: String, name: String, input: Value, status: String, summary: String },
}

/// Applies one streamed event to the steps.
pub fn apply(steps: &mut Vec<RunStep>, e: &agent::Event) {
    match e {
        agent::Event::Text { text } => match steps.last_mut() {
            Some(RunStep::Text { text: t }) => t.push_str(text),
            _ => steps.push(RunStep::Text { text: text.clone() }),
        },
        agent::Event::ToolStart { id, name, input } => steps.push(RunStep::Tool { id: id.clone(), name: name.clone(), input: input.clone(), status: "running".into(), summary: String::new() }),
        agent::Event::ToolEnd { id, ok, summary } => {
            for s in steps.iter_mut().rev() {
                if let RunStep::Tool { id: i, status, summary: sm, .. } = s {
                    if i == id {
                        *status = if *ok { "ok" } else { "error" }.into();
                        *sm = summary.clone();
                        break;
                    }
                }
            }
        }
        agent::Event::Approval { id, name, input } => steps.push(RunStep::Tool { id: id.clone(), name: name.clone(), input: input.clone(), status: "pending".into(), summary: String::new() }),
    }
}

// ---------------------------------------------------------------- runs

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub kind: String,
    pub title: String,
    /// queued | running | concluded | failed | cancelled
    pub status: String,
    /// manual | monitor | signal | schedule
    pub trigger: String,
    pub subject: Value,
    pub created_by: String,
    /// Epoch ms.
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub verdict: Option<String>,
    pub root_cause: Option<String>,
    pub error: Option<String>,
    pub feedback: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    #[serde(flatten)]
    pub summary: RunSummary,
    pub steps: Vec<RunStep>,
    pub conclusion: Option<Conclusion>,
    /// Coding sessions: the change proposed.
    pub change: Option<CodeChange>,
    /// Coding sessions: the pull request opened from it ({url, number, branch, draft, state}).
    pub pull_request: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    kind: String,
    title: String,
    status: String,
    trigger: String,
    subject: sqlx::types::Json<Value>,
    run_as: Option<String>,
    created_by: String,
    created_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    verdict: Option<String>,
    error: Option<String>,
    feedback: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    steps: sqlx::types::Json<Vec<RunStep>>,
    conclusion: Option<sqlx::types::Json<Conclusion>>,
    change: Option<sqlx::types::Json<CodeChange>>,
    pull_request: Option<sqlx::types::Json<Value>>,
}

const ROW: &str = "SELECT id::text, kind, title, status, trigger, subject, run_as::text, created_by, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM started_at) * 1000)::bigint AS started_at, \
    (extract(epoch FROM finished_at) * 1000)::bigint AS finished_at, verdict, error, feedback, input_tokens, output_tokens, steps, conclusion, change, pull_request FROM assist_runs";

impl Row {
    fn summary(&self) -> RunSummary {
        RunSummary {
            id: self.id.clone(),
            kind: self.kind.clone(),
            title: self.title.clone(),
            status: self.status.clone(),
            trigger: self.trigger.clone(),
            subject: self.subject.0.clone(),
            created_by: self.created_by.clone(),
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            verdict: self.verdict.clone(),
            root_cause: self.conclusion.as_ref().and_then(|c| c.0.root_cause.clone()),
            error: self.error.clone(),
            feedback: self.feedback.clone(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<Row, ApiError> {
    if !is_uuid(id) {
        return Err(ApiError::NotFound("No such run".into()));
    }
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!("{ROW} WHERE org_id = $1 AND id = $2::uuid")))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("No such run".into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartRun {
    /// investigation | security
    pub kind: String,
    /// Investigate this monitor (its latest alert).
    pub monitor_id: Option<i64>,
    /// Or investigate this, in the user's words.
    pub question: Option<String>,
    /// Judge this security signal.
    pub signal_id: Option<String>,
    /// Investigate this Error Tracking issue.
    #[serde(default)]
    pub issue_id: Option<String>,
}

/// An Error Tracking issue as the member sees it, over the last 14 days.
async fn error_issue(st: &AppState, ctx: &AuthContext, id: &str) -> Result<crate::routes::error_tracking::IssueDetailOut, ApiError> {
    let access = crate::log_access::for_user(st, ctx).await?;
    let now = chrono::Utc::now().timestamp_millis();
    let req = crate::routes::error_tracking::IssueDetailRequest { range: TimeRange { from: now - 14 * 86_400_000, to: now }, query: String::new() };
    Ok(crate::routes::error_tracking::issue_detail(State(st.clone()), OrgId(ctx.org_id.clone()), access, Path(id.to_string()), Json(req)).await?.0)
}

/// Runs created in the last hour (all kinds) and automatic ones in the last day, against the org's limits.
pub(crate) async fn within_limits(st: &AppState, org: &str) -> Result<(), ApiError> {
    crate::routes::assist_usage::check_credits(st, org, None).await?;
    let (hour, per_hour): (i64, i32) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM assist_runs WHERE org_id = $1 AND created_at > now() - interval '1 hour'), \
         COALESCE((SELECT runs_per_hour FROM assist_run_settings WHERE org_id = $1), 30)",
    )
    .bind(org)
    .fetch_one(&st.postgres)
    .await?;
    if hour >= per_hour as i64 {
        return Err(ApiError::TooManyRequests(format!("This organization started {hour} Assist runs in the last hour, its limit (Investigation → Settings → Rate Limits)")));
    }
    Ok(())
}

/// Starts an investigation (of a monitor's latest alert, or of a question) or a security verdict.
#[utoipa::path(post, path = "/api/v1/assist/runs", operation_id = "assist_start_run", request_body = StartRun, responses((status = 200, body = RunSummary)), tag = "assist")]
pub async fn start_run(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<StartRun>) -> ApiResult<RunSummary> {
    let kind = Kind::parse(&req.kind).filter(|k| matches!(k, Kind::Investigation | Kind::Security)).ok_or_else(|| ApiError::BadRequest("kind must be investigation or security".into()))?;
    let (title, subject) = match (kind, req.monitor_id, req.question.as_deref().map(str::trim).filter(|q| !q.is_empty()), req.signal_id.as_deref()) {
        (Kind::Investigation, None, _, _) if req.issue_id.is_some() => {
            let id = req.issue_id.clone().unwrap_or_default();
            let i = error_issue(&st, &ctx, &id).await?.issue;
            let message = i.message.lines().next().unwrap_or_default();
            let title = format!("{} in {}: {message}", i.error_type, i.service);
            (title.chars().take(140).collect(), json!({"issueId": id, "service": i.service}))
        }
        (Kind::Investigation, Some(id), _, _) => {
            let m = monitors::get_one(State(st.clone()), OrgId(ctx.org_id.clone()), Path(id)).await?.0;
            let alert: Option<(String, Option<f64>, i64)> = sqlx::query_as(
                "SELECT group_key, value, (extract(epoch FROM created_at) * 1000)::bigint FROM monitor_events \
                 WHERE monitor_id = $1 AND to_status = 'alert' ORDER BY created_at DESC LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&st.postgres)
            .await?;
            let mut subject = json!({"monitorId": id});
            if let Some((group, value, at)) = alert {
                subject["groupKey"] = json!(group);
                subject["value"] = json!(value);
                subject["alertAt"] = json!(at);
            }
            (m.summary.name, subject)
        }
        (Kind::Investigation, None, Some(q), _) if q.chars().count() <= 4000 => (q.lines().next().unwrap_or(q).chars().take(100).collect(), json!({"question": q})),
        (Kind::Security, _, _, Some(signal)) => {
            let s = crate::routes::security_signals::load_signal(&st, &ctx.org_id, signal).await?;
            (s.title.clone(), json!({"signalId": signal}))
        }
        _ => return Err(ApiError::BadRequest("Give a monitor, an error issue, a question (up to 4000 characters) or a signal".into())),
    };
    within_limits(&st, &ctx.org_id).await?;
    Ok(Json(insert_run(&st, &ctx.org_id, kind, &title, "manual", &subject, &ctx.user_id, &ctx.name).await?))
}

/// Queues a run and wakes the worker.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_run(st: &AppState, org: &str, kind: Kind, title: &str, trigger: &str, subject: &Value, run_as: &str, created_by: &str) -> Result<RunSummary, ApiError> {
    let id: String = sqlx::query_scalar("INSERT INTO assist_runs (org_id, kind, title, trigger, subject, run_as, created_by) VALUES ($1, $2, $3, $4, $5, $6::uuid, $7) RETURNING id::text")
        .bind(org)
        .bind(kind.as_str())
        .bind(title)
        .bind(trigger)
        .bind(sqlx::types::Json(subject))
        .bind(run_as)
        .bind(created_by)
        .fetch_one(&st.postgres)
        .await?;
    WAKE.notify_one();
    Ok(load(st, org, &id).await?.summary())
}

/// Queues an investigation of a monitor that just went to Alert, if the monitor investigates its alerts
/// automatically and the org is within its limits. Runs in the evaluator's transaction.
pub async fn enqueue_monitor_alert(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: &str, monitor_id: i64, name: &str, group: &str, value: Option<f64>) -> Result<bool, ApiError> {
    let subject = json!({"monitorId": monitor_id, "groupKey": group, "value": value, "alertAt": chrono::Utc::now().timestamp_millis()});
    let done = sqlx::query(
        "INSERT INTO assist_runs (org_id, kind, title, trigger, subject, run_as, created_by) \
         SELECT $1, 'investigation', $2, 'monitor', $3, s.enabled_by, 'Auto-investigate' FROM assist_monitor_settings s \
         LEFT JOIN assist_run_settings r ON r.org_id = s.org_id \
         WHERE s.org_id = $1 AND s.monitor_id = $4 AND s.auto_investigate AND s.enabled_by IS NOT NULL \
           AND (SELECT count(*) FROM assist_runs WHERE org_id = $1 AND trigger <> 'manual' AND created_at > now() - interval '1 day') < COALESCE(r.automatic_runs_per_day, 100) \
           AND (SELECT count(*) FROM assist_runs WHERE org_id = $1 AND created_at > now() - interval '1 hour') < COALESCE(r.runs_per_hour, 30) \
           AND NOT EXISTS (SELECT 1 FROM assist_runs WHERE org_id = $1 AND kind = 'investigation' AND status IN ('queued', 'running') \
                           AND subject->>'monitorId' = $4::text AND subject->>'groupKey' = $5)",
    )
    .bind(org)
    .bind(name)
    .bind(sqlx::types::Json(&subject))
    .bind(monitor_id)
    .bind(group)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if done > 0 {
        WAKE.notify_one();
    }
    Ok(done > 0)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunSearch {
    pub kind: String,
    pub monitor_id: Option<i64>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    /// Runs about this Error Tracking issue.
    #[serde(default)]
    pub issue_id: Option<String>,
}

#[utoipa::path(post, path = "/api/v1/assist/runs/search", operation_id = "assist_search_runs", request_body = RunSearch, responses((status = 200, body = Vec<RunSummary>)), tag = "assist")]
pub async fn search_runs(State(st): State<AppState>, ctx: AuthContext, Json(q): Json<RunSearch>) -> ApiResult<Vec<RunSummary>> {
    let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
        "{ROW} WHERE org_id = $1 AND kind = $2 AND ($3::bigint IS NULL OR subject->>'monitorId' = $3::text) AND ($4::text IS NULL OR status = $4) \
         AND ($6::text IS NULL OR subject->>'issueId' = $6) ORDER BY created_at DESC LIMIT $5"
    )))
    .bind(&ctx.org_id)
    .bind(&q.kind)
    .bind(q.monitor_id)
    .bind(&q.status)
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .bind(&q.issue_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows.iter().map(Row::summary).collect()))
}

#[utoipa::path(get, path = "/api/v1/assist/runs/{id}", operation_id = "assist_get_run", params(("id" = String, Path)), responses((status = 200, body = RunDetail)), tag = "assist")]
pub async fn get_run(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<RunDetail> {
    let row = load(&st, &ctx.org_id, &id).await?;
    Ok(Json(RunDetail {
        summary: row.summary(),
        steps: row.steps.0.clone(),
        conclusion: row.conclusion.as_ref().map(|c| c.0.clone()),
        change: row.change.as_ref().map(|c| c.0.clone()),
        pull_request: row.pull_request.as_ref().map(|p| p.0.clone()),
    }))
}

#[utoipa::path(post, path = "/api/v1/assist/runs/{id}/cancel", operation_id = "assist_cancel_run", params(("id" = String, Path)), responses((status = 204)), tag = "assist")]
pub async fn cancel_run(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE assist_runs SET status = 'cancelled', finished_at = now() WHERE org_id = $1 AND id = $2::uuid AND status IN ('queued', 'running')")
        .bind(&ctx.org_id)
        .bind(&id)
        .execute(&st.postgres)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunFeedback {
    /// helpful | not_helpful
    pub feedback: String,
    /// Kept as a memory for later investigations of the same monitor.
    pub remember: Option<String>,
}

#[utoipa::path(post, path = "/api/v1/assist/runs/{id}/feedback", operation_id = "assist_run_feedback", params(("id" = String, Path)), request_body = RunFeedback, responses((status = 204)), tag = "assist")]
pub async fn run_feedback(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(f): Json<RunFeedback>) -> Result<StatusCode, ApiError> {
    if !matches!(f.feedback.as_str(), "helpful" | "not_helpful") {
        return Err(ApiError::BadRequest("feedback is helpful or not_helpful".into()));
    }
    let row = load(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE assist_runs SET feedback = $3 WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).bind(&f.feedback).execute(&st.postgres).await?;
    if let Some(text) = f.remember.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        if ctx.access < Access::Standard {
            return Err(ApiError::Forbidden("Read-only members can't add memories".into()));
        }
        insert_memory(&st, &ctx, row.subject.0["monitorId"].as_i64(), text, Some(&id)).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- the worker

static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// Runs one server executes at once.
const SLOTS: usize = 4;
static RUNNING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(SLOTS);

/// Claims queued runs (any server may) and runs them; fails those whose server stopped.
pub async fn run_worker(st: AppState) {
    loop {
        if let Err(e) = sqlx::query("UPDATE assist_runs SET status = 'failed', error = 'The server running it stopped', finished_at = now() WHERE status = 'running' AND heartbeat_at < now() - interval '3 minutes'")
            .execute(&st.postgres)
            .await
        {
            tracing::warn!(error = %e, "assist runs: failing stale runs");
        }
        if let Err(e) = crate::routes::assist_agents::enqueue_due(&st).await {
            tracing::warn!(error = %e, "assist runs: scheduling agents");
        }
        let free = RUNNING.available_permits();
        match claim(&st, free).await {
            Ok(runs) => {
                for (org, id) in runs {
                    let st = st.clone();
                    let Ok(permit) = RUNNING.acquire().await else { break };
                    tokio::spawn(async move {
                        execute(&st, &org, &id).await;
                        drop(permit);
                    });
                }
            }
            Err(e) => tracing::warn!(error = %e, "assist runs: claiming"),
        }
        tokio::select! {
            _ = WAKE.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

async fn claim(st: &AppState, n: usize) -> Result<Vec<(String, String)>, sqlx::Error> {
    if n == 0 {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "UPDATE assist_runs SET status = 'running', started_at = now(), heartbeat_at = now() WHERE (org_id, id) IN \
         (SELECT org_id, id FROM assist_runs WHERE status = 'queued' ORDER BY created_at LIMIT $1 FOR UPDATE SKIP LOCKED) RETURNING org_id, id::text",
    )
    .bind(n as i64)
    .fetch_all(&st.postgres)
    .await
}

/// Claims and runs what the org has queued, one after the other (tests).
#[cfg(test)]
pub async fn work_once(st: &AppState, org: &str) -> usize {
    let runs: Vec<(String, String)> = sqlx::query_as(
        "UPDATE assist_runs SET status = 'running', started_at = now(), heartbeat_at = now() WHERE org_id = $1 AND status = 'queued' RETURNING org_id, id::text",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await
    .unwrap();
    for (org, id) in &runs {
        execute(st, org, id).await;
    }
    runs.len()
}

/// The opening message of a run: what it is about.
async fn opening(st: &AppState, ctx: &AuthContext, kind: &str, title: &str, subject: &Value) -> Result<String, ApiError> {
    if let Some(id) = subject["monitorId"].as_i64() {
        let m = monitors::get_one(State(st.clone()), OrgId(ctx.org_id.clone()), Path(id)).await?.0;
        let mut s = format!("Investigate why this monitor alerted.\n\nMonitor #{id}: {}\n{}", m.summary.name, agent::truncate(&serde_json::to_string_pretty(&m).unwrap_or_default(), 6000));
        if let Some(at) = subject["alertAt"].as_i64() {
            let when = chrono::DateTime::from_timestamp_millis(at).map(|d| d.to_rfc3339()).unwrap_or_default();
            s.push_str(&format!("\n\nIt went to ALERT at {when} for group `{}` (value {}).", subject["groupKey"].as_str().unwrap_or("*"), subject["value"]));
        }
        return Ok(s);
    }
    if let Some(signal) = subject["signalId"].as_str() {
        let s = crate::routes::security_signals::load_signal(st, &ctx.org_id, signal).await?;
        return Ok(format!("Triage this Cloud SIEM signal and give your verdict.\n\n{}", agent::truncate(&serde_json::to_string_pretty(&s).unwrap_or_default(), 8000)));
    }
    if let Some(id) = subject["issueId"].as_str() {
        let issue = error_issue(st, ctx, id).await?;
        return Ok(format!(
            "Investigate this Error Tracking issue: why it happens, since when, what changed (deploys, versions, dependencies), how many users or requests it affects, \
and where in the code it comes from.\n\n{}",
            agent::truncate(&serde_json::to_string_pretty(&issue).unwrap_or_default(), 8000)
        ));
    }
    if let Some(q) = subject["question"].as_str() {
        return Ok(format!("Investigate: {q}"));
    }
    if let Some(repo) = subject["repo"].as_str() {
        let task = subject["prompt"].as_str().unwrap_or(title);
        let input = subject["input"].as_str().map(|i| format!("\nInput: {i}")).unwrap_or_default();
        return Ok(format!("Repository: {repo}\nTask: {task}{input}\n\nRead the relevant code (list_repo_files, search_repo_code, read_repo_file), propose the change with propose_change, then conclude."));
    }
    if subject.get("agentId").is_some() {
        let when = chrono::Utc::now().to_rfc3339();
        return Ok(match subject["input"].as_str().map(str::trim).filter(|i| !i.is_empty()) {
            Some(input) => format!("Run now ({when}). Input from the person who started you:\n{input}"),
            None => format!("Run now ({when})."),
        });
    }
    Ok(format!("{kind}: {title}"))
}

struct Done {
    status: &'static str,
    conclusion: Option<Conclusion>,
    error: Option<String>,
    change: Option<CodeChange>,
}

/// Runs one claimed run to its end and stores what it did.
async fn execute(st: &AppState, org: &str, id: &str) {
    let steps: Arc<Mutex<Vec<RunStep>>> = Arc::default();
    let mut msgs: Vec<Msg> = Vec::new();
    let mut usage = Usage::default();
    let done = match run(st, org, id, &steps, &mut msgs, &mut usage).await {
        Ok(d) => d,
        Err(e) => Done { status: "failed", conclusion: None, error: Some(e), change: None },
    };
    let steps = steps.lock().unwrap().clone();
    let saved = sqlx::query(
        "UPDATE assist_runs SET status = CASE WHEN status = 'cancelled' THEN status ELSE $3 END, steps = $4, messages = $5, conclusion = $6, verdict = $7, \
         error = $8, input_tokens = input_tokens + $9, output_tokens = output_tokens + $10, change = $11, finished_at = COALESCE(finished_at, now()) WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(org)
    .bind(id)
    .bind(done.status)
    .bind(sqlx::types::Json(&steps))
    .bind(sqlx::types::Json(&msgs))
    .bind(done.conclusion.as_ref().map(sqlx::types::Json))
    .bind(done.conclusion.as_ref().map(|c| c.verdict.clone()))
    .bind(&done.error)
    .bind(usage.input as i64)
    .bind(usage.output as i64)
    .bind(done.change.as_ref().map(sqlx::types::Json))
    .execute(&st.postgres)
    .await;
    if let Err(e) = saved {
        tracing::warn!(error = %e, run = id, "assist runs: saving a run");
    }
    if let Some(change) = &done.change {
        auto_pull_request(st, org, id, change).await;
    }
    if let Some(c) = &done.conclusion {
        notify(st, org, id, c).await;
    }
}

/// Opens the pull request of a proposed change right away when the org asks for it.
async fn auto_pull_request(st: &AppState, org: &str, id: &str, change: &CodeChange) {
    let Ok(s) = crate::routes::assist_code::settings(st, org).await else { return };
    let Ok(row) = load(st, org, id).await else { return };
    let Some(repo) = row.subject.0["repo"].as_str() else { return };
    if s.auto_pull_request && row.status == "concluded" {
        if let Err(e) = crate::routes::assist_code::open_for_run(st, org, id, &row.title, repo, change).await {
            tracing::warn!(error = %e, run = id, "assist code: opening a pull request");
        }
    }
}

async fn run(st: &AppState, org: &str, id: &str, steps: &Arc<Mutex<Vec<RunStep>>>, msgs: &mut Vec<Msg>, usage: &mut Usage) -> Result<Done, String> {
    let e = |e: ApiError| e.to_string();
    let row = load(st, org, id).await.map_err(e)?;
    let kind = Kind::parse(&row.kind).ok_or("Unknown kind of run")?;
    let Some((ctx, _)) = (match &row.run_as {
        Some(u) => crate::routes::workflows::member_with(st, org, u, Access::ReadOnly).await.map_err(e)?,
        None => None,
    }) else {
        return Err("The member this run acts as is no longer active".into());
    };
    crate::routes::assist_usage::check_credits(st, org, None).await.map_err(e)?;
    let resolved = assist_provider::resolve(st, org).await.map_err(e)?.ok_or("Assist has no model: an admin sets one in Organization Settings → Assist AI")?;
    let max_tokens = st.config.assist_max_tokens.unwrap_or(match resolved.provider {
        Provider::Anthropic { .. } => 32_000,
        Provider::OpenAi { .. } => 4_096,
    });
    let model = crate::assist::http::HttpModel { provider: resolved.provider, max_tokens, reach: resolved.reach };
    let settings = run_settings(st, org).await.map_err(e)?;
    let memories: Vec<String> = sqlx::query_scalar(
        "SELECT text FROM assist_memories WHERE org_id = $1 AND (monitor_id IS NULL OR monitor_id = $2) ORDER BY monitor_id NULLS LAST, created_at DESC LIMIT 30",
    )
    .bind(org)
    .bind(row.subject.0["monitorId"].as_i64())
    .fetch_all(&st.postgres)
    .await
    .map_err(|x| x.to_string())?;
    let (instructions, disabled, actions) = match row.subject.0["agentId"].as_str() {
        Some(agent) => {
            let a = crate::routes::assist_agents::for_run(st, org, agent).await.map_err(e)?.ok_or("The agent or automation was deleted")?;
            let disabled = crate::assist::tools::SKILLS.iter().map(|s| s.name.to_string()).filter(|s| !a.skills.contains(s)).collect();
            (a.instructions, disabled, a.allow_actions)
        }
        None => (String::new(), Vec::new(), false),
    };
    let system = run_prompt(&RunPrompt { kind, today: &chrono::Utc::now().format("%Y-%m-%d").to_string(), knowledge: &settings.knowledge, memories: &memories, instructions: &instructions });
    msgs.push(Msg::User(opening(st, &ctx, &row.kind, &row.title, &row.subject.0).await.map_err(e)?));
    let tools = RunTools::new(OrgTools { st: st.clone(), ctx, disabled }, kind, actions);
    let l = Loop { model: &model, tools: &tools, system: &system, auto_approve: actions };
    let cancelled = AtomicBool::new(false);
    let outcome = {
        let sink = steps.clone();
        let mut emit = move |ev: agent::Event| apply(&mut sink.lock().unwrap(), &ev);
        let work = l.run(msgs, usage, &mut emit);
        let watch = watch(st, org, id, steps, &cancelled);
        tokio::select! {
            o = work => o,
            _ = watch => Outcome::Failed("cancelled".into()),
        }
    };
    if cancelled.load(Ordering::Relaxed) {
        return Ok(Done { status: "cancelled", conclusion: None, error: None, change: None });
    }
    let change = match (tools.change(), row.subject.0["repo"].as_str()) {
        (Some(c), Some(repo)) => Some(crate::routes::assist_code::with_before(st, org, repo, c).await),
        _ => None,
    };
    Ok(match (tools.conclusion(), outcome) {
        (Some(c), _) => Done { status: "concluded", conclusion: Some(c), error: None, change },
        (None, Outcome::Failed(why)) => Done { status: "failed", conclusion: None, error: Some(why), change },
        (None, _) => {
            // Answered without concluding: its last words are the summary.
            let text = msgs.iter().rev().find_map(|m| if let Msg::Assistant { text, .. } = m { (!text.trim().is_empty()).then(|| text.clone()) } else { None }).unwrap_or_default();
            let verdict = if kind == Kind::Security { "inconclusive" } else { kind.verdicts()[1] };
            Done { status: "concluded", conclusion: Some(Conclusion { summary: text, verdict: verdict.into(), confidence: "low".into(), ..Default::default() }), error: None, change }
        }
    })
}

/// Saves the steps and the heartbeat every two seconds; returns once the run is cancelled.
async fn watch(st: &AppState, org: &str, id: &str, steps: &Arc<Mutex<Vec<RunStep>>>, cancelled: &AtomicBool) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let snapshot = steps.lock().unwrap().clone();
        let status: Result<Option<String>, _> =
            sqlx::query_scalar("UPDATE assist_runs SET steps = $3, heartbeat_at = now() WHERE org_id = $1 AND id = $2::uuid RETURNING status").bind(org).bind(id).bind(sqlx::types::Json(&snapshot)).fetch_optional(&st.postgres).await;
        if matches!(status.as_ref().map(|s| s.as_deref()), Ok(Some("cancelled")) | Ok(None)) {
            cancelled.store(true, Ordering::Relaxed);
            return;
        }
    }
}

/// Tells the org's chosen channels that a run concluded.
async fn notify(st: &AppState, org: &str, id: &str, c: &Conclusion) {
    let Ok(row) = load(st, org, id).await else { return };
    let Ok(settings) = run_settings(st, org).await else { return };
    if settings.notify.is_empty() {
        return;
    }
    let path = if row.kind == "security" { "assist/security-analyst" } else { "assist/investigations" };
    let link = format!("{}/{path}/{id}", st.config.public_url);
    let title = format!("Assist {}: {}", c.verdict.replace('_', " "), row.title);
    let out = notifications::Outgoing {
        org: org.to_string(),
        title: title.clone(),
        email_body: format!("{title}\n\n{}\n\n{link}\n", c.summary),
        slack_text: format!("*{title}*\n{}\n<{link}|Open in Assist>", c.summary),
        webhook: json!({"event": format!("assist.{}.concluded", row.kind), "title": title, "organization": org, "run": {"id": id, "url": link}, "conclusion": c}),
    };
    for handle in &settings.notify {
        let handle = handle.trim_start_matches('@');
        if let notifications::Outcome::Failed(e) | notifications::Outcome::Unknown(e) = notifications::send(st, handle, &out).await {
            tracing::warn!(error = %e, handle, "assist runs: notifying");
        }
    }
}

// ---------------------------------------------------------------- security analyst

/// Queues a verdict on a new signal when the org judges signals of its severity automatically.
pub async fn enqueue_signal(st: &AppState, org: &str, signal: &str, title: &str, severity: &str) -> Result<bool, ApiError> {
    let done = sqlx::query(
        "INSERT INTO assist_runs (org_id, kind, title, trigger, subject, run_as, created_by) \
         SELECT $1, 'security', $2, 'signal', jsonb_build_object('signalId', $3::text), r.security_run_as, 'Security Analyst' FROM assist_run_settings r \
         WHERE r.org_id = $1 AND $4 = ANY(r.security_auto_severities) AND r.security_run_as IS NOT NULL \
           AND (SELECT count(*) FROM assist_runs WHERE org_id = $1 AND trigger <> 'manual' AND created_at > now() - interval '1 day') < r.automatic_runs_per_day \
           AND (SELECT count(*) FROM assist_runs WHERE org_id = $1 AND created_at > now() - interval '1 hour') < r.runs_per_hour \
           AND NOT EXISTS (SELECT 1 FROM assist_runs WHERE org_id = $1 AND kind = 'security' AND subject->>'signalId' = $3)",
    )
    .bind(org)
    .bind(title)
    .bind(signal)
    .bind(severity)
    .execute(&st.postgres)
    .await?
    .rows_affected();
    if done > 0 {
        WAKE.notify_one();
    }
    Ok(done > 0)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalsRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzedSignal {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub state: String,
    pub rule_name: String,
    pub created_at: i64,
    /// The latest verdict run on it.
    pub run: Option<RunSummary>,
}

/// Signals in the range, newest first, with Assist's latest verdict on each.
#[utoipa::path(post, path = "/api/v1/assist/security/signals", operation_id = "assist_analyzed_signals", request_body = SignalsRequest, responses((status = 200, body = Vec<AnalyzedSignal>)), tag = "assist")]
pub async fn analyzed_signals(State(st): State<AppState>, ctx: AuthContext, Json(r): Json<SignalsRequest>) -> ApiResult<Vec<AnalyzedSignal>> {
    let signals: Vec<(String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, title, severity, state, rule_name, (extract(epoch FROM created_at) * 1000)::bigint FROM security_signals \
         WHERE org_id = $1 AND created_at >= to_timestamp($2::float8 / 1000) AND created_at < to_timestamp($3::float8 / 1000) ORDER BY created_at DESC LIMIT 500",
    )
    .bind(&ctx.org_id)
    .bind(r.range.from)
    .bind(r.range.to)
    .fetch_all(&st.postgres)
    .await?;
    let ids: Vec<String> = signals.iter().map(|s| s.0.clone()).collect();
    let runs = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
        "{ROW} WHERE org_id = $1 AND kind = 'security' AND subject->>'signalId' = ANY($2) ORDER BY created_at DESC"
    )))
    .bind(&ctx.org_id)
    .bind(&ids)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(
        signals
            .into_iter()
            .map(|(id, title, severity, state, rule_name, created_at)| AnalyzedSignal {
                run: runs.iter().find(|r| r.subject.0["signalId"].as_str() == Some(id.as_str())).map(Row::summary),
                id,
                title,
                severity,
                state,
                rule_name,
                created_at,
            })
            .collect(),
    ))
}

// ---------------------------------------------------------------- settings, memories, monitors

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RunSettings {
    /// The knowledge base every run reads (Markdown).
    pub knowledge: String,
    pub runs_per_hour: i32,
    pub automatic_runs_per_day: i32,
    /// Channels (`@slack-ops`, an email) told when a run concludes.
    pub notify: Vec<String>,
    /// Signals of these severities are judged automatically.
    pub security_auto_severities: Vec<String>,
    /// The member automatic security runs act as.
    pub security_run_as: Option<String>,
}

async fn run_settings(st: &AppState, org: &str) -> Result<RunSettings, ApiError> {
    Ok(sqlx::query_as::<_, RunSettings>("SELECT knowledge, runs_per_hour, automatic_runs_per_day, notify, security_auto_severities, security_run_as::text AS security_run_as FROM assist_run_settings WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?
        .unwrap_or(RunSettings { knowledge: String::new(), runs_per_hour: 30, automatic_runs_per_day: 100, notify: Vec::new(), security_auto_severities: Vec::new(), security_run_as: None }))
}

#[utoipa::path(get, path = "/api/v1/assist/run-settings", operation_id = "assist_get_run_settings", responses((status = 200, body = RunSettings)), tag = "assist")]
pub async fn get_run_settings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<RunSettings> {
    Ok(Json(run_settings(&st, &ctx.org_id).await?))
}

const SEVERITIES: &[&str] = &["info", "low", "medium", "high", "critical"];

#[utoipa::path(put, path = "/api/v1/assist/run-settings", operation_id = "assist_put_run_settings", request_body = RunSettings, responses((status = 204)), tag = "assist")]
pub async fn put_run_settings(State(st): State<AppState>, Admin(ctx): Admin, Json(s): Json<RunSettings>) -> Result<StatusCode, ApiError> {
    if s.knowledge.chars().count() > 20_000 {
        return Err(ApiError::BadRequest("The knowledge base is limited to 20000 characters".into()));
    }
    if !(1..=1000).contains(&s.runs_per_hour) || !(0..=10_000).contains(&s.automatic_runs_per_day) {
        return Err(ApiError::BadRequest("Runs per hour is 1 to 1000; automatic runs per day 0 to 10000".into()));
    }
    if let Some(bad) = s.security_auto_severities.iter().find(|v| !SEVERITIES.contains(&v.as_str())) {
        return Err(ApiError::BadRequest(format!("Unknown severity {bad}")));
    }
    let notify: Vec<String> = s.notify.iter().map(|h| h.trim().trim_start_matches('@').to_string()).filter(|h| !h.is_empty()).take(20).collect();
    let run_as = match (&s.security_run_as, s.security_auto_severities.is_empty()) {
        (Some(u), _) if is_uuid(u) => Some(u.clone()),
        (_, false) => Some(ctx.user_id.clone()),
        _ => None,
    };
    sqlx::query(
        "INSERT INTO assist_run_settings (org_id, knowledge, runs_per_hour, automatic_runs_per_day, notify, security_auto_severities, security_run_as, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, now()) ON CONFLICT (org_id) DO UPDATE SET knowledge = $2, runs_per_hour = $3, automatic_runs_per_day = $4, \
         notify = $5, security_auto_severities = $6, security_run_as = $7::uuid, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(s.knowledge.trim())
    .bind(s.runs_per_hour)
    .bind(s.automatic_runs_per_day)
    .bind(&notify)
    .bind(&s.security_auto_severities)
    .bind(run_as)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "assist.run_settings.update", "", &format!("{} runs/hour, {} automatic/day", s.runs_per_hour, s.automatic_runs_per_day)).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub monitor_id: Option<i64>,
    pub monitor_name: Option<String>,
    pub text: String,
    pub source_run: Option<String>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    /// For this monitor's investigations; none for every run.
    pub monitor_id: Option<i64>,
    pub text: String,
}

async fn insert_memory(st: &AppState, ctx: &AuthContext, monitor: Option<i64>, text: &str, source: Option<&str>) -> Result<(), ApiError> {
    if text.chars().count() > 2000 {
        return Err(ApiError::BadRequest("A memory is up to 2000 characters".into()));
    }
    if let Some(id) = monitor {
        let _ = monitors::get_one(State(st.clone()), OrgId(ctx.org_id.clone()), Path(id)).await?;
    }
    sqlx::query("INSERT INTO assist_memories (org_id, monitor_id, text, source_run, created_by) VALUES ($1, $2, $3, $4::uuid, $5)")
        .bind(&ctx.org_id)
        .bind(monitor)
        .bind(text)
        .bind(source)
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?;
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/assist/memories", operation_id = "assist_list_memories", responses((status = 200, body = Vec<Memory>)), tag = "assist")]
pub async fn list_memories(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<Memory>> {
    type MemoryRow = (String, Option<i64>, Option<String>, String, Option<String>, String, i64);
    let rows: Vec<MemoryRow> = sqlx::query_as(
        "SELECT a.id::text, a.monitor_id, m.name, a.text, a.source_run::text, a.created_by, (extract(epoch FROM a.created_at) * 1000)::bigint \
         FROM assist_memories a LEFT JOIN monitors m ON m.id = a.monitor_id AND m.org_id = a.org_id WHERE a.org_id = $1 ORDER BY a.created_at DESC",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows.into_iter().map(|(id, monitor_id, monitor_name, text, source_run, created_by, created_at)| Memory { id, monitor_id, monitor_name, text, source_run, created_by, created_at }).collect()))
}

#[utoipa::path(post, path = "/api/v1/assist/memories", operation_id = "assist_add_memory", request_body = NewMemory, responses((status = 204)), tag = "assist")]
pub async fn add_memory(State(st): State<AppState>, Writer(ctx): Writer, Json(m): Json<NewMemory>) -> Result<StatusCode, ApiError> {
    let text = m.text.trim();
    if text.is_empty() {
        return Err(ApiError::BadRequest("Write what to remember".into()));
    }
    insert_memory(&st, &ctx, m.monitor_id, text, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/assist/memories/{id}", operation_id = "assist_delete_memory", params(("id" = String, Path)), responses((status = 204)), tag = "assist")]
pub async fn delete_memory(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) {
        return Err(ApiError::NotFound("No such memory".into()));
    }
    sqlx::query("DELETE FROM assist_memories WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecentAlert {
    pub at: i64,
    pub group: String,
    pub value: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationMonitor {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub priority: Option<i16>,
    pub tags: Vec<String>,
    pub teams: Vec<String>,
    /// Its data can be investigated (synthetic tests can't yet).
    pub supported: bool,
    pub auto_investigate: bool,
    pub investigations: i64,
    pub memories: i64,
    /// Alerts in the last 7 days, newest first (at most 5).
    pub recent_alerts: Vec<RecentAlert>,
}

/// Every monitor with what Assist Investigation knows of it.
#[utoipa::path(get, path = "/api/v1/assist/investigations/monitors", operation_id = "assist_investigation_monitors", responses((status = 200, body = Vec<InvestigationMonitor>)), tag = "assist")]
pub async fn investigation_monitors(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<InvestigationMonitor>> {
    type MonitorRow = (i64, String, String, String, Option<i16>, Vec<String>, Vec<String>, bool, i64, i64);
    let rows: Vec<MonitorRow> = sqlx::query_as(
        "SELECT m.id, m.name, m.type, m.status, m.priority, m.tags, m.teams, COALESCE(s.auto_investigate, false), \
           (SELECT count(*) FROM assist_runs r WHERE r.org_id = m.org_id AND r.kind = 'investigation' AND r.subject->>'monitorId' = m.id::text), \
           (SELECT count(*) FROM assist_memories a WHERE a.org_id = m.org_id AND a.monitor_id = m.id) \
         FROM monitors m LEFT JOIN assist_monitor_settings s ON s.org_id = m.org_id AND s.monitor_id = m.id \
         WHERE m.org_id = $1 AND NOT m.draft ORDER BY m.name",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    let alerts: Vec<(i64, i64, String, Option<f64>)> = sqlx::query_as(
        "SELECT e.monitor_id, (extract(epoch FROM e.created_at) * 1000)::bigint, e.group_key, e.value FROM monitor_events e JOIN monitors m ON m.id = e.monitor_id \
         WHERE m.org_id = $1 AND e.to_status = 'alert' AND e.created_at > now() - interval '7 days' ORDER BY e.created_at DESC LIMIT 2000",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, kind, status, priority, tags, teams, auto_investigate, investigations, memories)| InvestigationMonitor {
                supported: kind != "synthetics",
                recent_alerts: alerts.iter().filter(|a| a.0 == id).take(5).map(|a| RecentAlert { at: a.1, group: a.2.clone(), value: a.3 }).collect(),
                id,
                name,
                kind,
                status,
                priority,
                tags,
                teams,
                auto_investigate,
                investigations,
                memories,
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorAuto {
    pub auto_investigate: bool,
}

/// Turns automatic investigation of a monitor's alerts on or off; runs act as the member who turned it on.
#[utoipa::path(put, path = "/api/v1/assist/investigations/monitors/{id}", operation_id = "assist_set_monitor_auto", params(("id" = i64, Path)), request_body = MonitorAuto, responses((status = 204)), tag = "assist")]
pub async fn set_monitor_auto(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i64>, Json(m): Json<MonitorAuto>) -> Result<StatusCode, ApiError> {
    let monitor = monitors::get_one(State(st.clone()), OrgId(ctx.org_id.clone()), Path(id)).await?.0;
    sqlx::query(
        "INSERT INTO assist_monitor_settings (org_id, monitor_id, auto_investigate, enabled_by, updated_at) VALUES ($1, $2, $3, $4::uuid, now()) \
         ON CONFLICT (org_id, monitor_id) DO UPDATE SET auto_investigate = $3, enabled_by = $4::uuid, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(m.auto_investigate)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, if m.auto_investigate { "assist.auto_investigate.enable" } else { "assist.auto_investigate.disable" }, &format!("monitor:{id}"), &monitor.summary.name).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- reports

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunReportRequest {
    pub range: TimeRange,
    /// investigation | security | agent | code
    pub kind: String,
    pub monitor_id: Option<i64>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunCount {
    pub name: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TimeCount {
    pub at: i64,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub total: i64,
    pub concluded: i64,
    pub failed: i64,
    /// From start to conclusion, on average.
    pub mean_seconds_to_conclusion: Option<f64>,
    pub helpful: i64,
    pub not_helpful: i64,
    pub over_time: Vec<TimeCount>,
    pub by_trigger: Vec<RunCount>,
    pub by_verdict: Vec<RunCount>,
    /// Investigations: by monitor name.
    pub by_monitor: Vec<RunCount>,
}

const REPORT_FILTER: &str = "org_id = $1 AND kind = $2 AND created_at >= to_timestamp($3::float8 / 1000) AND created_at < to_timestamp($4::float8 / 1000) AND ($5::bigint IS NULL OR subject->>'monitorId' = $5::text)";

#[utoipa::path(post, path = "/api/v1/assist/runs/report", operation_id = "assist_run_report", request_body = RunReportRequest, responses((status = 200, body = RunReport)), tag = "assist")]
pub async fn investigation_report(State(st): State<AppState>, ctx: AuthContext, Json(r): Json<RunReportRequest>) -> ApiResult<RunReport> {
    if r.range.to <= r.range.from {
        return Err(ApiError::BadRequest("The range ends before it starts".into()));
    }
    let bind = |q: &'static str| -> String { q.replace("{F}", REPORT_FILTER) };
    macro_rules! q {
        ($t:ty, $sql:expr) => {
            sqlx::query_as::<_, $t>(sqlx::AssertSqlSafe(bind($sql))).bind(&ctx.org_id).bind(&r.kind).bind(r.range.from).bind(r.range.to).bind(r.monitor_id)
        };
    }
    let (total, concluded, failed, mean, helpful, not_helpful): (i64, i64, i64, Option<f64>, i64, i64) = q!(
        (i64, i64, i64, Option<f64>, i64, i64),
        "SELECT count(*), count(*) FILTER (WHERE status = 'concluded'), count(*) FILTER (WHERE status = 'failed'), \
         avg(extract(epoch FROM finished_at - created_at)) FILTER (WHERE status = 'concluded')::float8, \
         count(*) FILTER (WHERE feedback = 'helpful'), count(*) FILTER (WHERE feedback = 'not_helpful') FROM assist_runs WHERE {F}"
    )
    .fetch_one(&st.postgres)
    .await?;
    let over_time: Vec<(i64, i64)> = q!(
        (i64, i64),
        "SELECT (extract(epoch FROM date_trunc(CASE WHEN $4 - $3 <= 172800000 THEN 'hour' ELSE 'day' END, created_at)) * 1000)::bigint AS at, count(*) FROM assist_runs WHERE {F} GROUP BY 1 ORDER BY 1"
    )
    .fetch_all(&st.postgres)
    .await?;
    let named = |rows: Vec<(String, i64)>| rows.into_iter().map(|(name, count)| RunCount { name, count }).collect::<Vec<_>>();
    let by_trigger = named(q!((String, i64), "SELECT trigger, count(*) FROM assist_runs WHERE {F} GROUP BY 1 ORDER BY 2 DESC, 1").fetch_all(&st.postgres).await?);
    let by_verdict = named(q!((String, i64), "SELECT COALESCE(verdict, status), count(*) FROM assist_runs WHERE {F} GROUP BY 1 ORDER BY 2 DESC, 1").fetch_all(&st.postgres).await?);
    let by_monitor = named(
        q!(
            (String, i64),
            "SELECT COALESCE(m.name, r.title), count(*) FROM (SELECT org_id, title, subject FROM assist_runs WHERE {F} AND subject ? 'monitorId') r \
             LEFT JOIN monitors m ON m.org_id = r.org_id AND m.id::text = r.subject->>'monitorId' GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 20"
        )
        .fetch_all(&st.postgres)
        .await?,
    );
    Ok(Json(RunReport { total, concluded, failed, mean_seconds_to_conclusion: mean, helpful, not_helpful, over_time: over_time.into_iter().map(|(at, count)| TimeCount { at, count }).collect(), by_trigger, by_verdict, by_monitor }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::testing::{fake_anthropic, sse_text, sse_tool};
    use crate::auth::Writer;
    use serde_json::json;

    #[test]
    fn steps_follow_the_streamed_events() {
        let mut steps = Vec::new();
        apply(&mut steps, &agent::Event::Text { text: "Looking at ".into() });
        apply(&mut steps, &agent::Event::Text { text: "the logs.".into() });
        apply(&mut steps, &agent::Event::ToolStart { id: "t1".into(), name: "search_logs".into(), input: json!({"query": "status:error"}) });
        apply(&mut steps, &agent::Event::ToolEnd { id: "t1".into(), ok: true, summary: "{\"total\":3}".into() });
        apply(&mut steps, &agent::Event::Text { text: "Found it.".into() });
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], RunStep::Text { text: "Looking at the logs.".into() });
        assert!(matches!(&steps[1], RunStep::Tool { status, summary, .. } if status == "ok" && summary == "{\"total\":3}"));
    }

    /// An investigation of a monitor: queued, run by the worker as its member with the knowledge base and
    /// the monitor's memories, concluded with evidence, reported; automatic runs follow the monitor's
    /// setting and the rate limit holds. Skipped unless OC_TEST_POSTGRES_URL and OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn an_investigation_runs_in_the_background_and_concludes() {
        let conclusion = json!({"summary": "inventory-service timed out after its deploy", "verdict": "conclusive", "confidence": "high", "root_cause": "inventory-service 3.2.1", "evidence": [{"title": "412 timeouts", "detail": "order-service logs", "link": "/logs?query=service%3Aorder-service"}], "next_steps": ["Roll back inventory-service"]});
        let (base_url, seen) = fake_anthropic(vec![sse_tool("t1", "search_logs", json!({"query": "service:order-service status:error"})), sse_tool("t2", "conclude", conclusion), sse_text("Concluded.")]).await;
        let Some((_, states)) = crate::test_support::replicas_with(1, |cfg| {
            cfg.assist = Ok(Some(Provider::Anthropic { base_url: base_url.clone(), api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }));
        })
        .await
        else {
            return;
        };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "runs").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roles (id, org_id, name, description, managed) VALUES ($1 || '-admin', $1, 'Admin', '', true) ON CONFLICT DO NOTHING").bind(&admin.org_id).execute(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2)").bind(&user_id).bind(format!("{}-admin", admin.org_id)).execute(&st.postgres).await.unwrap();
        let ctx = AuthContext { user_id, ..admin };
        let monitor: monitors::MonitorInput = serde_json::from_value(json!({"name": "High error rate on order-service", "type": "log", "query": "logs(\"service:order-service status:error\") by {service}", "message": "", "options": {"detection": "threshold", "timeAggregation": "sum", "windowMinutes": 5, "comparator": "above", "thresholds": {"critical": 10}}})).unwrap();
        let monitor_id = monitors::create(State(st.clone()), Writer(ctx.clone()), Json(monitor)).await.unwrap().0.summary.id;

        put_run_settings(State(st.clone()), Admin(ctx.clone()), Json(RunSettings { knowledge: "order-service is owned by @team-orders".into(), runs_per_hour: 2, automatic_runs_per_day: 1, notify: vec![], security_auto_severities: vec![], security_run_as: None })).await.unwrap();
        add_memory(State(st.clone()), Writer(ctx.clone()), Json(NewMemory { monitor_id: Some(monitor_id), text: "Spikes at 02:00 are the nightly batch".into() })).await.unwrap();

        // Started by hand: queued, then run by the worker.
        let run = start_run(State(st.clone()), ctx.clone(), Json(StartRun { kind: "investigation".into(), monitor_id: Some(monitor_id), question: None, signal_id: None, issue_id: None })).await.unwrap().0;
        assert_eq!((run.status.as_str(), run.trigger.as_str()), ("queued", "manual"));
        assert_eq!(work_once(&st, &ctx.org_id).await, 1);
        let r = get_run(State(st.clone()), ctx.clone(), Path(run.id.clone())).await.unwrap().0;
        assert_eq!((r.summary.status.as_str(), r.summary.verdict.as_deref()), ("concluded", Some("conclusive")), "{:?}", r.summary.error);
        let c = r.conclusion.as_ref().unwrap();
        assert_eq!((c.root_cause.as_deref(), c.evidence.len()), (Some("inventory-service 3.2.1"), 1));
        assert!(r.steps.iter().any(|s| matches!(s, RunStep::Tool { name, status, .. } if name == "search_logs" && status == "ok")), "{:?}", r.steps);
        assert!(r.summary.input_tokens > 0);
        let system = seen.lock().unwrap()[0]["system"].as_str().unwrap().to_string();
        assert!(system.contains("@team-orders") && system.contains("nightly batch"), "{system}");
        let first = seen.lock().unwrap()[0]["messages"][0]["content"].to_string();
        assert!(first.contains("High error rate on order-service"), "{first}");

        // Listed for the monitor, and reported.
        let monitors = investigation_monitors(State(st.clone()), ctx.clone()).await.unwrap().0;
        let m = monitors.iter().find(|m| m.id == monitor_id).unwrap();
        assert_eq!((m.supported, m.investigations, m.memories, m.auto_investigate), (true, 1, 1, false));
        let now = chrono::Utc::now().timestamp_millis();
        let rep = investigation_report(State(st.clone()), ctx.clone(), Json(RunReportRequest { range: TimeRange { from: now - 3_600_000, to: now + 60_000 }, kind: "investigation".into(), monitor_id: None })).await.unwrap().0;
        assert_eq!((rep.total, rep.concluded), (1, 1));
        assert_eq!(rep.by_trigger, vec![RunCount { name: "manual".into(), count: 1 }]);

        // Automatic: only for monitors that ask for it, within the daily limit.
        let mut tx = st.postgres.begin().await.unwrap();
        assert!(!enqueue_monitor_alert(&mut tx, &ctx.org_id, monitor_id, "High error rate on order-service", "service:order-service", Some(42.0)).await.unwrap());
        tx.commit().await.unwrap();
        set_monitor_auto(State(st.clone()), Writer(ctx.clone()), Path(monitor_id), Json(MonitorAuto { auto_investigate: true })).await.unwrap();
        let mut tx = st.postgres.begin().await.unwrap();
        assert!(enqueue_monitor_alert(&mut tx, &ctx.org_id, monitor_id, "High error rate on order-service", "service:order-service", Some(42.0)).await.unwrap());
        assert!(!enqueue_monitor_alert(&mut tx, &ctx.org_id, monitor_id, "High error rate on order-service", "service:order-service", Some(43.0)).await.unwrap(), "one a day");
        tx.commit().await.unwrap();
        let listed = search_runs(State(st.clone()), ctx.clone(), Json(RunSearch { kind: "investigation".into(), monitor_id: Some(monitor_id), status: None, limit: None, issue_id: None })).await.unwrap().0;
        assert_eq!(listed.iter().map(|r| r.trigger.as_str()).collect::<Vec<_>>(), vec!["monitor", "manual"]);

        // An Error Tracking issue that isn't there can't be investigated; runs are listed per issue.
        let missing = StartRun { kind: "investigation".into(), monitor_id: None, question: None, signal_id: None, issue_id: Some("0123456789abcdef".into()) };
        assert!(matches!(start_run(State(st.clone()), ctx.clone(), Json(missing)).await, Err(ApiError::NotFound(_))));
        let none = search_runs(State(st.clone()), ctx.clone(), Json(RunSearch { kind: "investigation".into(), monitor_id: None, status: None, limit: None, issue_id: Some("0123456789abcdef".into()) })).await.unwrap().0;
        assert!(none.is_empty());

        // Two an hour: the third is refused.
        assert!(matches!(start_run(State(st.clone()), ctx.clone(), Json(StartRun { kind: "investigation".into(), monitor_id: None, question: Some("Why is checkout slow?".into()), signal_id: None, issue_id: None })).await, Err(ApiError::TooManyRequests(_))));

        // Cancelled before it ran.
        let queued = listed[0].id.clone();
        cancel_run(State(st.clone()), Writer(ctx.clone()), Path(queued.clone())).await.unwrap();
        assert_eq!(get_run(State(st.clone()), ctx.clone(), Path(queued)).await.unwrap().0.summary.status, "cancelled");
    }

    /// A new signal of a chosen severity is judged automatically, as the member who chose it, and listed
    /// with its verdict. Skipped unless OC_TEST_POSTGRES_URL and OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn new_signals_get_a_verdict() {
        let verdict = json!({"summary": "Logins from the corporate VPN after an MFA timeout", "verdict": "benign", "confidence": "high", "evidence": [{"title": "Known VPN range"}]});
        let (base_url, seen) = fake_anthropic(vec![sse_tool("t1", "get_security_signal", json!({"id": "SIG-TEST-1"})), sse_tool("t2", "conclude", verdict), sse_text("Benign.")]).await;
        let Some((_, states)) = crate::test_support::replicas_with(1, |cfg| {
            cfg.assist = Ok(Some(Provider::Anthropic { base_url: base_url.clone(), api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }));
        })
        .await
        else {
            return;
        };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "secrun").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let ctx = AuthContext { user_id, ..admin };
        sqlx::query("INSERT INTO security_signals (org_id, id, rule_id, rule_name, rule_type, title, message, severity, first_seen, last_seen) VALUES ($1, 'SIG-TEST-1', 'r1', 'MFA fatigue', 'log_detection', 'MFA push denied then accepted for alex@example.com', '', 'high', now(), now())")
            .bind(&ctx.org_id)
            .execute(&st.postgres)
            .await
            .unwrap();

        // Not chosen: nothing happens.
        assert!(!enqueue_signal(&st, &ctx.org_id, "SIG-TEST-1", "MFA push denied then accepted for alex@example.com", "high").await.unwrap());
        put_run_settings(State(st.clone()), Admin(ctx.clone()), Json(RunSettings { knowledge: String::new(), runs_per_hour: 30, automatic_runs_per_day: 100, notify: vec![], security_auto_severities: vec!["high".into(), "critical".into()], security_run_as: None })).await.unwrap();
        assert!(!enqueue_signal(&st, &ctx.org_id, "SIG-TEST-1", "x", "low").await.unwrap());
        assert!(enqueue_signal(&st, &ctx.org_id, "SIG-TEST-1", "MFA push denied then accepted for alex@example.com", "high").await.unwrap());
        assert_eq!(work_once(&st, &ctx.org_id).await, 1);
        let first = seen.lock().unwrap()[0]["messages"][0]["content"].to_string();
        assert!(first.contains("MFA fatigue"), "{first}");

        let now = chrono::Utc::now().timestamp_millis();
        let listed = analyzed_signals(State(st.clone()), ctx.clone(), Json(SignalsRequest { range: TimeRange { from: now - 3_600_000, to: now + 60_000 } })).await.unwrap().0;
        assert_eq!(listed.len(), 1);
        let run = listed[0].run.as_ref().unwrap();
        assert_eq!((run.status.as_str(), run.verdict.as_deref(), run.trigger.as_str()), ("concluded", Some("benign"), "signal"));
    }
}
