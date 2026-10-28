//! Automation → Workflow Automation: workflows, their triggers and their runs.
//!
//! - **Workflows** are edited as a draft (`spec`) and run by their triggers once published
//!   (`published_spec`); see [`super::workflow_engine`] for what they contain. Standard users and
//!   admins create, edit, publish and run them; everyone in the organization sees them and their runs.
//! - **Triggers.** A manual run (editor or list) runs the saved draft as the member who clicks. The
//!   published workflow is started by a monitor mentioning `@workflow-<handle>` in its message (on the
//!   states the trigger lists), by its cron schedule, by an incident declared, updated or resolved, by
//!   `POST /api/v1/workflows/{id}/trigger` (an application key plus the workflow's own token in
//!   `X-Workflow-Token`, kept hashed) or by an app's button. Those runs act as the member who
//!   published it, who must still be a standard user or admin.
//! - **Runs** execute in the background: a run is claimed with `FOR UPDATE SKIP LOCKED` and leased,
//!   its state is saved after every step and every step is recorded (rendered inputs, output,
//!   error, duration; credentials redacted). A step is bounded to 5 minutes including its retries,
//!   a run to 500 action steps and 6 hours (waits included). HTTP requests go out through the
//!   synthetic tests' client, which refuses private and internal addresses unless the operator
//!   allows them. Runs started by a run (an incident declared by a step) are refused past 3 levels.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{Notify, Semaphore};
use utoipa::ToSchema;

use super::portal_action_steps::{self as steps, ActionStep, RenderContext};
use super::portal_actions::{merge_secrets, run_http, run_internal};
use super::workflow_engine::{self as engine, Next, RunState, WorkflowSpec, WorkflowStep, WorkflowStepKind, WorkflowTriggerKind};
use crate::auth::{access_from_roles, Access, AuthContext, Credential, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_WORKFLOWS: i64 = 500;
const MAX_TAGS: usize = 20;
/// Runs of one workflow queued, running or waiting at once.
const MAX_ACTIVE_RUNS: i64 = 50;
const RUNS_LISTED: i64 = 100;
const STEP_TIMEOUT: Duration = Duration::from_secs(300);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RUN_HOURS: i64 = 6;
/// Runs one server executes at once.
const CONCURRENT_RUNS: usize = 16;
const LEASE_SECONDS: f64 = 600.0;
/// Larger step outputs are cut to an excerpt.
const MAX_OUTPUT_CHARS: usize = 32_000;
const MAX_RECIPIENTS: usize = 20;
/// Runs started by runs (a declared incident starting another workflow) stop at this depth.
const MAX_CHAIN: u8 = 3;
const RUN_RETENTION_DAYS: i32 = 30;

tokio::task_local! {
    /// Set while a run executes a step: how deep in a chain of runs it is.
    static CHAIN: u8;
}

// ================================================================ API types

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInput {
    pub name: String,
    /// Mentioned as `@workflow-<handle>`; made from the name when empty at creation, kept otherwise.
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Team handle; empty for none.
    #[serde(default)]
    pub owner_team: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub blueprint: String,
    pub spec: WorkflowSpec,
    /// Credentials by name: a value sets one, null removes it, names left out keep their value.
    #[serde(default)]
    pub secrets: BTreeMap<String, Option<String>>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowLastRun {
    pub id: i64,
    pub status: String,
    pub trigger_kind: String,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub handle: String,
    pub description: String,
    pub tags: Vec<String>,
    pub owner_team: String,
    pub service: String,
    pub blueprint: String,
    /// The saved draft.
    pub spec: WorkflowSpec,
    /// What triggers run; none until published.
    pub published_spec: Option<WorkflowSpec>,
    pub published: bool,
    /// The draft differs from the published version.
    pub unpublished_changes: bool,
    pub published_at: Option<i64>,
    pub published_by: Option<String>,
    /// Names of the stored credentials (values are never returned).
    pub secret_names: Vec<String>,
    /// An API trigger token exists.
    pub has_api_token: bool,
    pub next_schedule_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub runs: i64,
    pub last_run: Option<WorkflowLastRun>,
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
pub struct WorkflowRunInput {
    /// Value per input key.
    #[serde(default)]
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub inputs: Map<String, Value>,
}

#[derive(Serialize, ToSchema, Clone, Debug, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunStep {
    pub id: i64,
    pub step_id: String,
    pub kind: String,
    pub name: String,
    /// The "for each" item it ran for, when inside one.
    pub iteration: Option<i32>,
    /// succeeded | failed
    pub status: String,
    #[schema(value_type = Object)]
    pub inputs: Value,
    #[schema(value_type = Object)]
    pub output: Value,
    pub error: Option<String>,
    pub attempts: i32,
    pub started_at: i64,
    pub duration_ms: f64,
}

#[derive(Serialize, ToSchema, Clone, Debug, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: i64,
    pub workflow_id: String,
    pub workflow_name: String,
    /// manual | monitor | schedule | incident | api | app
    pub trigger_kind: String,
    pub triggered_by: String,
    /// queued | running | waiting | succeeded | failed | canceled
    pub status: String,
    pub error: Option<String>,
    #[schema(value_type = Object)]
    pub trigger: Value,
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub inputs: Value,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// Waiting: when it goes on.
    pub next_at: Option<i64>,
    /// Detail only: what each step did, in order.
    #[sqlx(skip)]
    pub steps: Vec<WorkflowRunStep>,
    /// Detail only: the spec it ran.
    #[sqlx(skip)]
    pub spec: Option<WorkflowSpec>,
}

#[derive(Serialize, ToSchema)]
pub struct WorkflowApiToken {
    /// Send it as `X-Workflow-Token`; shown once.
    pub token: String,
}

// ================================================================ checks

fn text(value: &str, what: &str, max: usize, required: bool) -> Result<String, String> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(format!("{what} is required"));
    }
    if v.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok(v.to_string())
}

/// `Restart order-service when error rate alerts` → `restart-order-service-when-error-rate-alerts`.
pub fn handle_from(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(60).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "workflow".into()
    } else {
        out
    }
}

fn handle_ok(h: &str) -> bool {
    !h.is_empty() && h.len() <= 60 && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn team_ok(h: &str) -> bool {
    h.is_empty() || (h.len() <= 100 && h.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
}

#[derive(Debug)]
struct Checked {
    name: String,
    handle: String,
    description: String,
    tags: Vec<String>,
    owner_team: String,
    service: String,
    blueprint: String,
    spec: WorkflowSpec,
    secrets: BTreeMap<String, String>,
}

fn check_workflow(input: &WorkflowInput, current_handle: Option<&str>, current_secrets: &BTreeMap<String, String>) -> Result<Checked, String> {
    let name = text(&input.name, "The name", 200, true)?;
    let handle = match input.handle.trim() {
        "" => current_handle.map(str::to_string).unwrap_or_else(|| handle_from(&name)),
        h => h.trim_start_matches('@').trim_start_matches("workflow-").to_lowercase(),
    };
    if !handle_ok(&handle) {
        return Err(format!("{handle} isn't a handle (a–z, 0–9, - and _, up to 60)"));
    }
    let description = text(&input.description, "The description", 2000, false)?;
    if input.tags.len() > MAX_TAGS {
        return Err(format!("A workflow has at most {MAX_TAGS} tags"));
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &input.tags {
        let t = text(t, "A tag", 100, true)?;
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    let owner_team = input.owner_team.trim().trim_start_matches('@').to_string();
    if !team_ok(&owner_team) {
        return Err(format!("{owner_team} isn't a team handle"));
    }
    let service = text(&input.service, "The service", 200, false)?;
    let blueprint = text(&input.blueprint, "The blueprint", 100, false)?;
    let secrets = merge_secrets(current_secrets, &input.secrets)?;
    let names: HashSet<String> = secrets.keys().cloned().collect();
    let spec = engine::check_spec(&input.spec, &names)?;
    Ok(Checked { name, handle, description, tags, owner_team, service, blueprint, spec, secrets })
}

/// `@slack-checkout, oncall@example.com @webhook-ops` → handles and addresses.
pub fn recipient_list(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in s.split(|c: char| c == ',' || c.is_whitespace()).map(|r| r.trim().trim_start_matches('@')).filter(|r| !r.is_empty()) {
        if !out.iter().any(|x| x == r) {
            out.push(r.to_string());
        }
    }
    out
}

/// Keeps an output small enough for the run's history and state.
pub fn cap_output(v: Value) -> Value {
    let text = v.to_string();
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return v;
    }
    json!({"truncated": true, "excerpt": text.chars().take(2000).collect::<String>()})
}

fn redact_value(v: &Value, secrets: &BTreeMap<String, String>) -> Value {
    if secrets.is_empty() {
        return v.clone();
    }
    let text = steps::redact(&v.to_string(), secrets);
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn redact_opt(s: Option<String>, secrets: &BTreeMap<String, String>) -> Option<String> {
    s.map(|e| steps::redact(&e, secrets))
}

// ================================================================ storage

fn valid_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn not_found() -> ApiError {
    ApiError::NotFound("Workflow not found".into())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

fn json_of<T: Serialize>(v: &T) -> Result<Value, ApiError> {
    serde_json::to_value(v).map_err(|e| ApiError::Internal(e.into()))
}

#[derive(sqlx::FromRow)]
struct WorkflowRow {
    id: String,
    name: String,
    handle: String,
    description: String,
    tags: Vec<String>,
    owner_team: String,
    service: String,
    blueprint: String,
    spec: Value,
    published_spec: Option<Value>,
    published_at: Option<i64>,
    published_by: Option<String>,
    secrets_enc: Option<String>,
    has_api_token: bool,
    next_schedule_at: Option<i64>,
    created_by: String,
    created_at: i64,
    updated_at: i64,
    runs: i64,
    last_run_id: Option<i64>,
    last_run_status: Option<String>,
    last_run_kind: Option<String>,
    last_run_at: Option<i64>,
}

fn spec_of(v: &Value) -> Result<WorkflowSpec, ApiError> {
    serde_json::from_value(v.clone()).map_err(|e| ApiError::Internal(anyhow::anyhow!("stored workflow unreadable: {e}")))
}

impl WorkflowRow {
    fn secrets(&self, st: &AppState) -> Result<BTreeMap<String, String>, ApiError> {
        let Some(enc) = &self.secrets_enc else { return Ok(BTreeMap::new()) };
        let plain = st.secrets.decrypt(enc).map_err(|_| ApiError::BadRequest("The workflow's secrets can't be decrypted (was OC_SECRET_KEY changed?); enter them again".into()))?;
        serde_json::from_str(&plain).map_err(|e| ApiError::Internal(anyhow::anyhow!("stored workflow secrets unreadable: {e}")))
    }

    fn view(&self, st: &AppState) -> Result<Workflow, ApiError> {
        let spec = spec_of(&self.spec)?;
        let published_spec = self.published_spec.as_ref().map(spec_of).transpose()?;
        Ok(Workflow {
            id: self.id.clone(),
            name: self.name.clone(),
            handle: self.handle.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
            owner_team: self.owner_team.clone(),
            service: self.service.clone(),
            blueprint: self.blueprint.clone(),
            published: published_spec.is_some(),
            unpublished_changes: published_spec.as_ref().is_some_and(|p| *p != spec),
            spec,
            published_spec,
            published_at: self.published_at,
            published_by: self.published_by.clone(),
            secret_names: self.secrets(st).map(|s| s.into_keys().collect()).unwrap_or_default(),
            has_api_token: self.has_api_token,
            next_schedule_at: self.next_schedule_at,
            created_by: self.created_by.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            runs: self.runs,
            last_run: match (self.last_run_id, &self.last_run_status, &self.last_run_kind, self.last_run_at) {
                (Some(id), Some(status), Some(kind), Some(at)) => Some(WorkflowLastRun { id, status: status.clone(), trigger_kind: kind.clone(), created_at: at }),
                _ => None,
            },
        })
    }
}

const MS: &str = "(extract(epoch FROM {}) * 1000)::bigint";

fn ms(col: &str) -> String {
    MS.replace("{}", col)
}

async fn workflow_rows(st: &AppState, org: &str, id: Option<&str>) -> Result<Vec<WorkflowRow>, ApiError> {
    let sql = format!(
        "SELECT w.id::text, w.name, w.handle, w.description, w.tags, w.owner_team, w.service, w.blueprint, w.spec, w.published_spec, \
         {published_at} AS published_at, p.name AS published_by, w.secrets_enc, (w.api_token_hash IS NOT NULL) AS has_api_token, \
         {next_at} AS next_schedule_at, COALESCE(u.name, '') AS created_by, {created} AS created_at, {updated} AS updated_at, \
         (SELECT count(*) FROM workflow_runs r WHERE r.org_id = w.org_id AND r.workflow_id = w.id) AS runs, \
         l.id AS last_run_id, l.status AS last_run_status, l.trigger_kind AS last_run_kind, {last_at} AS last_run_at \
         FROM workflows w LEFT JOIN users u ON u.id = w.created_by LEFT JOIN users p ON p.id = w.published_by \
         LEFT JOIN LATERAL (SELECT r.id, r.status, r.trigger_kind, r.created_at FROM workflow_runs r WHERE r.org_id = w.org_id AND r.workflow_id = w.id ORDER BY r.created_at DESC, r.id DESC LIMIT 1) l ON true \
         WHERE w.org_id = $1 AND ($2::text IS NULL OR w.id::text = $2) ORDER BY w.name",
        published_at = ms("w.published_at"),
        next_at = ms("w.next_schedule_at"),
        created = ms("w.created_at"),
        updated = ms("w.updated_at"),
        last_at = ms("l.created_at"),
    );
    Ok(sqlx::query_as(&sql).bind(org).bind(id).fetch_all(&st.postgres).await?)
}

async fn workflow_row(st: &AppState, org: &str, id: &str) -> Result<WorkflowRow, ApiError> {
    if !valid_uuid(id) {
        return Err(not_found());
    }
    workflow_rows(st, org, Some(id)).await?.pop().ok_or_else(not_found)
}

fn seal(st: &AppState, secrets: &BTreeMap<String, String>) -> Result<Option<String>, ApiError> {
    if secrets.is_empty() {
        return Ok(None);
    }
    Ok(Some(st.secrets.encrypt(&serde_json::to_string(secrets).map_err(anyhow::Error::from)?)))
}

fn unique_violation(e: sqlx::Error, name: &str, handle: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            if d.constraint().is_some_and(|c| c.contains("handle")) {
                ApiError::BadRequest(format!("Another workflow uses the handle @workflow-{handle}"))
            } else {
                ApiError::BadRequest(format!("A workflow named {name} exists already"))
            }
        }
        _ => e.into(),
    }
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/workflows", operation_id = "list_workflows", responses((status = 200, body = Vec<Workflow>)), tag = "automation")]
pub async fn list_workflows(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<Workflow>> {
    let rows = workflow_rows(&st, &ctx.org_id, None).await?;
    Ok(Json(rows.iter().map(|r| r.view(&st)).collect::<Result<_, _>>()?))
}

#[utoipa::path(get, path = "/api/v1/workflows/{id}", operation_id = "get_workflow", params(("id" = String, Path)),
    responses((status = 200, body = Workflow), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn get_workflow(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Workflow> {
    Ok(Json(workflow_row(&st, &ctx.org_id, &id).await?.view(&st)?))
}

#[utoipa::path(post, path = "/api/v1/workflows", operation_id = "create_workflow", request_body = WorkflowInput,
    responses((status = 200, body = Workflow), (status = 400, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn create_workflow(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<WorkflowInput>) -> ApiResult<Workflow> {
    let w = check_workflow(&input, None, &BTreeMap::new()).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM workflows WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_WORKFLOWS {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_WORKFLOWS} workflows")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO workflows (org_id, name, handle, description, tags, owner_team, service, blueprint, spec, secrets_enc, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, (SELECT id FROM users WHERE org_id = $1 AND id::text = $11)) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&w.name)
    .bind(&w.handle)
    .bind(&w.description)
    .bind(&w.tags)
    .bind(&w.owner_team)
    .bind(&w.service)
    .bind(&w.blueprint)
    .bind(json_of(&w.spec)?)
    .bind(seal(&st, &w.secrets)?)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| unique_violation(e, &w.name, &w.handle))?;
    ctx.audit(&st, "workflow.created", &id, &format!("{} ({} steps)", w.name, w.spec.steps.len())).await;
    get_workflow(State(st), ctx, Path(id)).await
}

#[utoipa::path(put, path = "/api/v1/workflows/{id}", operation_id = "update_workflow", params(("id" = String, Path)), request_body = WorkflowInput,
    responses((status = 200, body = Workflow), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn update_workflow(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<WorkflowInput>) -> ApiResult<Workflow> {
    let row = workflow_row(&st, &ctx.org_id, &id).await?;
    let current = row.secrets(&st).unwrap_or_default();
    let w = check_workflow(&input, Some(&row.handle), &current).map_err(ApiError::BadRequest)?;
    sqlx::query(
        "UPDATE workflows SET name = $3, handle = $4, description = $5, tags = $6, owner_team = $7, service = $8, blueprint = $9, spec = $10, secrets_enc = $11, updated_at = now() \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&w.name)
    .bind(&w.handle)
    .bind(&w.description)
    .bind(&w.tags)
    .bind(&w.owner_team)
    .bind(&w.service)
    .bind(&w.blueprint)
    .bind(json_of(&w.spec)?)
    .bind(seal(&st, &w.secrets)?)
    .execute(&st.postgres)
    .await
    .map_err(|e| unique_violation(e, &w.name, &w.handle))?;
    let changed: Vec<&str> = input.secrets.keys().map(String::as_str).collect();
    let detail = if changed.is_empty() { w.name.clone() } else { format!("{} (secrets changed: {})", w.name, changed.join(", ")) };
    ctx.audit(&st, "workflow.updated", &id, &detail).await;
    get_workflow(State(st), ctx, Path(id)).await
}

#[utoipa::path(delete, path = "/api/v1/workflows/{id}", operation_id = "delete_workflow", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn delete_workflow(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row = workflow_row(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM workflows WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "workflow.deleted", &id, &row.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Publishes the saved draft: triggers run it from now on, as the signed-in member.
#[utoipa::path(post, path = "/api/v1/workflows/{id}/publish", operation_id = "publish_workflow", params(("id" = String, Path)),
    responses((status = 200, body = Workflow), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn publish_workflow(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<Workflow> {
    let row = workflow_row(&st, &ctx.org_id, &id).await?;
    let names: HashSet<String> = row.secrets(&st)?.into_keys().collect();
    // Checked again: secrets may have been removed since it was saved.
    let spec = engine::check_spec(&spec_of(&row.spec)?, &names).map_err(ApiError::BadRequest)?;
    if spec.steps.is_empty() {
        return Err(ApiError::BadRequest("Add a step before publishing".into()));
    }
    let next = engine::next_schedule(&spec, now_ms());
    sqlx::query(
        "UPDATE workflows SET published_spec = $3, published_at = now(), published_by = (SELECT id FROM users WHERE org_id = $1 AND id::text = $4), \
         next_schedule_at = CASE WHEN $5::bigint IS NULL THEN NULL ELSE to_timestamp($5::bigint / 1000.0) END WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(json_of(&spec)?)
    .bind(&ctx.user_id)
    .bind(next)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "workflow.published", &id, &row.name).await;
    get_workflow(State(st), ctx, Path(id)).await
}

#[utoipa::path(post, path = "/api/v1/workflows/{id}/unpublish", operation_id = "unpublish_workflow", params(("id" = String, Path)),
    responses((status = 200, body = Workflow), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn unpublish_workflow(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<Workflow> {
    let row = workflow_row(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE workflows SET published_spec = NULL, published_at = NULL, published_by = NULL, next_schedule_at = NULL WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "workflow.unpublished", &id, &row.name).await;
    get_workflow(State(st), ctx, Path(id)).await
}

/// Creates a new API trigger token (the previous one stops working). It is shown once.
#[utoipa::path(post, path = "/api/v1/workflows/{id}/token", operation_id = "create_workflow_token", params(("id" = String, Path)),
    responses((status = 200, body = WorkflowApiToken), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn create_token(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<WorkflowApiToken> {
    let row = workflow_row(&st, &ctx.org_id, &id).await?;
    let token = crate::auth::crypto::random_token("ocwt_");
    sqlx::query("UPDATE workflows SET api_token_hash = $3 WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).bind(crate::auth::crypto::sha256(&token)).execute(&st.postgres).await?;
    ctx.audit(&st, "workflow.token_created", &id, &row.name).await;
    Ok(Json(WorkflowApiToken { token }))
}

/// Runs the saved draft now, as the signed-in member. The run executes in the background.
#[utoipa::path(post, path = "/api/v1/workflows/{id}/runs", operation_id = "run_workflow", params(("id" = String, Path)), request_body = WorkflowRunInput,
    responses((status = 200, body = WorkflowRun), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn run_workflow(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<WorkflowRunInput>) -> ApiResult<WorkflowRun> {
    let row = workflow_row(&st, &ctx.org_id, &id).await?;
    let spec = spec_of(&row.spec)?;
    if spec.steps.is_empty() {
        return Err(ApiError::BadRequest("Add a step before running the workflow".into()));
    }
    let inputs = steps::check_run_inputs(&spec.inputs, &input.inputs).map_err(ApiError::BadRequest)?;
    let trigger = json!({"kind": "manual", "user": {"name": ctx.name}});
    let run = enqueue(&st, &ctx.org_id, &row.id, &spec, Start { kind: WorkflowTriggerKind::Manual, by: ctx.name.clone(), run_as: Some(ctx.user_id.clone()), trigger, inputs }).await?;
    ctx.audit(&st, "workflow.run", &row.id, &format!("{}: run #{run}", row.name)).await;
    get_run(State(st), ctx, Path((id, run))).await
}

/// Starts the published workflow from a script: an application key authenticates the call and the
/// workflow's token (`X-Workflow-Token`) proves it may start this one.
#[utoipa::path(post, path = "/api/v1/workflows/{id}/trigger", operation_id = "trigger_workflow", params(("id" = String, Path)), request_body = WorkflowRunInput,
    responses((status = 200, body = WorkflowRun), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn trigger_workflow(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, headers: HeaderMap, Json(input): Json<WorkflowRunInput>) -> ApiResult<WorkflowRun> {
    if !valid_uuid(&id) {
        return Err(not_found());
    }
    let token = headers.get("x-workflow-token").and_then(|v| v.to_str().ok()).map(str::trim).unwrap_or("");
    type TriggerRow = (Option<Value>, Option<Vec<u8>>, Option<String>, String);
    let row: Option<TriggerRow> = sqlx::query_as("SELECT published_spec, api_token_hash, published_by::text, name FROM workflows WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .fetch_optional(&st.postgres)
        .await?;
    let Some((published, hash, publisher, name)) = row else { return Err(not_found()) };
    let expected = hash.ok_or_else(|| ApiError::Forbidden("Create an API token for this workflow first".into()))?;
    if token.is_empty() || !constant_time_eq(&crate::auth::crypto::sha256(token), &expected) {
        return Err(ApiError::Forbidden("The X-Workflow-Token header is missing or wrong".into()));
    }
    let spec = published.as_ref().map(spec_of).transpose()?.ok_or_else(|| ApiError::BadRequest("The workflow isn't published".into()))?;
    if !spec.has_trigger(WorkflowTriggerKind::Api) {
        return Err(ApiError::BadRequest("The workflow has no API trigger".into()));
    }
    let inputs = steps::check_run_inputs(&spec.inputs, &input.inputs).map_err(ApiError::BadRequest)?;
    let trigger = json!({"kind": "api", "user": {"name": ctx.name}});
    let run = enqueue(&st, &ctx.org_id, &id, &spec, Start { kind: WorkflowTriggerKind::Api, by: format!("API ({})", ctx.name), run_as: publisher, trigger, inputs }).await?;
    ctx.audit(&st, "workflow.triggered", &id, &format!("{name}: run #{run} (API)")).await;
    get_run(State(st), ctx, Path((id, run))).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

const RUN_COLUMNS: &str = "r.id, r.workflow_id::text, w.name AS workflow_name, r.trigger_kind, r.triggered_by, r.status, r.error, r.trigger, r.inputs, \
    (extract(epoch FROM r.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM r.started_at) * 1000)::bigint AS started_at, \
    (extract(epoch FROM r.finished_at) * 1000)::bigint AS finished_at, \
    CASE WHEN r.status = 'waiting' THEN (extract(epoch FROM r.next_at) * 1000)::bigint END AS next_at";

#[utoipa::path(get, path = "/api/v1/workflows/{id}/runs", operation_id = "list_workflow_runs", params(("id" = String, Path)),
    responses((status = 200, body = Vec<WorkflowRun>), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn list_runs(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Vec<WorkflowRun>> {
    if !valid_uuid(&id) {
        return Err(not_found());
    }
    let runs = sqlx::query_as(&format!(
        "SELECT {RUN_COLUMNS} FROM workflow_runs r JOIN workflows w ON w.org_id = r.org_id AND w.id = r.workflow_id \
         WHERE r.org_id = $1 AND r.workflow_id::text = $2 ORDER BY r.created_at DESC, r.id DESC LIMIT $3"
    ))
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(RUNS_LISTED)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(runs))
}

#[utoipa::path(get, path = "/api/v1/workflows/{id}/runs/{run}", operation_id = "get_workflow_run", params(("id" = String, Path), ("run" = i64, Path)),
    responses((status = 200, body = WorkflowRun), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn get_run(State(st): State<AppState>, ctx: AuthContext, Path((id, run)): Path<(String, i64)>) -> ApiResult<WorkflowRun> {
    Ok(Json(run_detail(&st, &ctx.org_id, &id, run).await?))
}

pub(crate) async fn run_detail(st: &AppState, org: &str, workflow: &str, run: i64) -> Result<WorkflowRun, ApiError> {
    let missing = || ApiError::NotFound("Run not found".into());
    if !valid_uuid(workflow) {
        return Err(missing());
    }
    let mut r: WorkflowRun = sqlx::query_as(&format!(
        "SELECT {RUN_COLUMNS} FROM workflow_runs r JOIN workflows w ON w.org_id = r.org_id AND w.id = r.workflow_id WHERE r.org_id = $1 AND r.workflow_id::text = $2 AND r.id = $3"
    ))
    .bind(org)
    .bind(workflow)
    .bind(run)
    .fetch_optional(&st.postgres)
    .await?
    .ok_or_else(missing)?;
    r.steps = sqlx::query_as(
        "SELECT id, step_id, kind, name, iteration, status, inputs, output, error, attempts, (extract(epoch FROM started_at) * 1000)::bigint AS started_at, duration_ms \
         FROM workflow_run_steps WHERE org_id = $1 AND run_id = $2 ORDER BY id",
    )
    .bind(org)
    .bind(run)
    .fetch_all(&st.postgres)
    .await?;
    let (spec,): (Value,) = sqlx::query_as("SELECT spec FROM workflow_runs WHERE org_id = $1 AND id = $2").bind(org).bind(run).fetch_one(&st.postgres).await?;
    r.spec = serde_json::from_value(spec).ok();
    Ok(r)
}

#[utoipa::path(post, path = "/api/v1/workflows/{id}/runs/{run}/cancel", operation_id = "cancel_workflow_run", params(("id" = String, Path), ("run" = i64, Path)),
    responses((status = 200, body = WorkflowRun), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn cancel_run(State(st): State<AppState>, Writer(ctx): Writer, Path((id, run)): Path<(String, i64)>) -> ApiResult<WorkflowRun> {
    let r = run_detail(&st, &ctx.org_id, &id, run).await?;
    sqlx::query("UPDATE workflow_runs SET status = 'canceled', finished_at = now(), lease_until = NULL, error = $3 WHERE org_id = $1 AND id = $2 AND status IN ('queued', 'running', 'waiting')")
        .bind(&ctx.org_id)
        .bind(run)
        .bind(format!("Canceled by {}", ctx.name))
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "workflow.run_canceled", &id, &format!("{}: run #{run}", r.workflow_name)).await;
    get_run(State(st), ctx, Path((id, run))).await
}

// ================================================================ starting runs

pub(crate) struct Start {
    pub kind: WorkflowTriggerKind,
    pub by: String,
    pub run_as: Option<String>,
    pub trigger: Value,
    pub inputs: Map<String, Value>,
}

fn chain_depth() -> u8 {
    CHAIN.try_with(|d| *d).unwrap_or(0)
}

/// Queues a run of `spec`; this server's executor starts it right away.
pub(crate) async fn enqueue(st: &AppState, org: &str, workflow: &str, spec: &WorkflowSpec, start: Start) -> Result<i64, ApiError> {
    let depth = chain_depth();
    if depth >= MAX_CHAIN {
        return Err(ApiError::BadRequest(format!("Runs started by other runs stop after {MAX_CHAIN} levels")));
    }
    let (active,): (i64,) = sqlx::query_as("SELECT count(*) FROM workflow_runs WHERE org_id = $1 AND workflow_id::text = $2 AND status IN ('queued', 'running', 'waiting')")
        .bind(org)
        .bind(workflow)
        .fetch_one(&st.postgres)
        .await?;
    if active >= MAX_ACTIVE_RUNS {
        return Err(ApiError::TooManyRequests(format!("The workflow has {MAX_ACTIVE_RUNS} runs in progress already")));
    }
    let mut trigger = start.trigger;
    if let Some(t) = trigger.as_object_mut() {
        t.insert("chain".into(), json!(depth));
    }
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO workflow_runs (org_id, workflow_id, trigger_kind, triggered_by, run_as, trigger, inputs, spec) \
         VALUES ($1, $2::uuid, $3, $4, (SELECT id FROM users WHERE org_id = $1 AND id::text = $5), $6, $7, $8) RETURNING id",
    )
    .bind(org)
    .bind(workflow)
    .bind(start.kind.as_str())
    .bind(start.by.chars().take(200).collect::<String>())
    .bind(start.run_as)
    .bind(&trigger)
    .bind(Value::Object(start.inputs))
    .bind(json_of(spec)?)
    .fetch_one(&st.postgres)
    .await?;
    wake().notify_one();
    Ok(id)
}

/// Wakes this server's executor for a run just queued.
fn wake() -> &'static Notify {
    static W: std::sync::OnceLock<Notify> = std::sync::OnceLock::new();
    W.get_or_init(Notify::new)
}

#[derive(sqlx::FromRow)]
struct Published {
    id: String,
    published_spec: Value,
    published_by: Option<String>,
}

/// The published workflows of an org that have a trigger of `kind`.
async fn published_with(st: &AppState, org: &str, kind: WorkflowTriggerKind) -> Result<Vec<(Published, WorkflowSpec)>, ApiError> {
    let rows: Vec<Published> = sqlx::query_as(
        "SELECT id::text, published_spec, published_by::text FROM workflows WHERE org_id = $1 AND published_spec IS NOT NULL \
         AND published_spec->'triggers' @> jsonb_build_array(jsonb_build_object('kind', $2::text))",
    )
    .bind(org)
    .bind(kind.as_str())
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().filter_map(|r| spec_of(&r.published_spec).ok().map(|s| (r, s))).collect())
}

/// A monitor notification addressed to `@workflow-<handle>` (from the notifier).
pub(crate) async fn start_from_monitor(st: &AppState, handle: &str, n: &super::notifications::Notice) -> super::notifications::Outcome {
    use super::notifications::Outcome;
    let row: Option<(String, Option<Value>, Option<String>)> =
        match sqlx::query_as("SELECT id::text, published_spec, published_by::text FROM workflows WHERE org_id = $1 AND handle = $2").bind(&n.org).bind(handle).fetch_optional(&st.postgres).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e.to_string()),
        };
    let Some((id, published, publisher)) = row else {
        return Outcome::Unknown(format!("No workflow has the handle @workflow-{handle}"));
    };
    let Some(spec) = published.as_ref().and_then(|p| spec_of(p).ok()) else {
        return Outcome::Unknown(format!("@workflow-{handle} isn't published"));
    };
    let Some(trigger) = spec.triggers.iter().find(|t| t.kind == WorkflowTriggerKind::Monitor) else {
        return Outcome::Unknown(format!("@workflow-{handle} has no monitor trigger"));
    };
    if n.test {
        return Outcome::Sent;
    }
    if !engine::monitor_matches(trigger, &n.to) {
        // Not a state this workflow runs on: nothing to deliver.
        return Outcome::Sent;
    }
    let payload = json!({
        "kind": "monitor",
        "monitor": {"id": n.monitor_id, "name": n.monitor_name, "priority": n.priority, "tags": n.tags, "url": format!("{}/monitors/{}", st.config.public_url, n.monitor_id)},
        "group": n.group,
        "transition": {"from": n.from, "to": n.to},
        "value": n.value,
        "message": n.message,
        "at": n.at,
    });
    let start = Start { kind: WorkflowTriggerKind::Monitor, by: format!("Monitor #{}", n.monitor_id), run_as: publisher, trigger: payload, inputs: Map::new() };
    match enqueue(st, &n.org, &id, &spec, start).await {
        Ok(_) => Outcome::Sent,
        Err(ApiError::TooManyRequests(m)) => Outcome::Failed(m),
        Err(ApiError::BadRequest(m)) => Outcome::Unknown(m),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// An incident was declared, updated or resolved: starts the workflows whose incident trigger matches.
pub(crate) async fn on_incident(st: &AppState, org: &str, event: &str, incident: &super::incidents::IncidentSummary) {
    let result: Result<(), ApiError> = async {
        let payload_incident = json_of(incident)?;
        for (row, spec) in published_with(st, org, WorkflowTriggerKind::Incident).await? {
            if !spec.triggers.iter().any(|t| engine::incident_matches(t, event, &incident.severity)) {
                continue;
            }
            let payload = json!({"kind": "incident", "event": event, "incident": payload_incident, "url": format!("{}/incidents/{}", st.config.public_url, incident.id)});
            let start = Start { kind: WorkflowTriggerKind::Incident, by: format!("Incident IR-{}", incident.id), run_as: row.published_by.clone(), trigger: payload, inputs: Map::new() };
            if let Err(e) = enqueue(st, org, &row.id, &spec, start).await {
                tracing::warn!(error = %e, workflow = row.id, "incident trigger not started");
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, "incident workflow triggers failed");
    }
}

/// Runs the published workflow for an app's button; returns the run id.
pub(crate) async fn start_from_app(st: &AppState, ctx: &AuthContext, workflow: &str, app: &str, inputs: &Map<String, Value>) -> Result<i64, ApiError> {
    if !valid_uuid(workflow) {
        return Err(not_found());
    }
    let row: Option<(Option<Value>, Option<String>)> =
        sqlx::query_as("SELECT published_spec, published_by::text FROM workflows WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(workflow).fetch_optional(&st.postgres).await?;
    let (published, publisher) = row.ok_or_else(not_found)?;
    let spec = published.as_ref().map(spec_of).transpose()?.ok_or_else(|| ApiError::BadRequest("The workflow this button runs isn't published".into()))?;
    if !spec.has_trigger(WorkflowTriggerKind::App) {
        return Err(ApiError::BadRequest("The workflow this button runs has no app trigger".into()));
    }
    // Values for inputs the workflow doesn't have are dropped.
    let known: Map<String, Value> = inputs.iter().filter(|(k, _)| spec.inputs.iter().any(|i| &i.key == *k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let inputs = steps::check_run_inputs(&spec.inputs, &known).map_err(ApiError::BadRequest)?;
    let trigger = json!({"kind": "app", "app": app, "user": {"name": ctx.name}});
    enqueue(st, &ctx.org_id, workflow, &spec, Start { kind: WorkflowTriggerKind::App, by: format!("{} (app)", ctx.name), run_as: publisher, trigger, inputs }).await
}

// ================================================================ executor

/// Starts scheduled runs and executes due runs every 2 seconds; also drops runs older than 30 days.
pub async fn run_executor(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut last_cleanup = Instant::now() - Duration::from_secs(3600);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = wake().notified() => {}
        }
        if let Err(e) = schedule_tick(&st).await {
            tracing::warn!(error = %e, "workflow schedule tick failed");
        }
        if let Err(e) = claim_and_run(&st).await {
            tracing::warn!(error = %e, "workflow executor tick failed");
        }
        if last_cleanup.elapsed() > Duration::from_secs(3600) {
            last_cleanup = Instant::now();
            let r = sqlx::query("DELETE FROM workflow_runs WHERE created_at < now() - make_interval(days => $1) AND status IN ('succeeded', 'failed', 'canceled')")
                .bind(RUN_RETENTION_DAYS)
                .execute(&st.postgres)
                .await;
            if let Err(e) = r {
                tracing::warn!(error = %e, "workflow run cleanup failed");
            }
        }
    }
}

async fn schedule_tick(st: &AppState) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    let due: Vec<(String, String, Value, Option<String>, i64)> = sqlx::query_as(
        "SELECT org_id, id::text, published_spec, published_by::text, (extract(epoch FROM next_schedule_at) * 1000)::bigint FROM workflows \
         WHERE next_schedule_at <= now() AND published_spec IS NOT NULL ORDER BY next_schedule_at LIMIT 20 FOR UPDATE SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut starts = Vec::new();
    for (org, id, published, publisher, at) in due {
        let spec = spec_of(&published)?;
        // Missed times (the server was down) aren't caught up: the next one is after now.
        let next = engine::next_schedule(&spec, now_ms());
        sqlx::query("UPDATE workflows SET next_schedule_at = CASE WHEN $3::bigint IS NULL THEN NULL ELSE to_timestamp($3::bigint / 1000.0) END WHERE org_id = $1 AND id::text = $2")
            .bind(&org)
            .bind(&id)
            .bind(next)
            .execute(&mut *tx)
            .await?;
        starts.push((org, id, spec, publisher, at));
    }
    tx.commit().await?;
    for (org, id, spec, publisher, at) in starts {
        let trigger = json!({"kind": "schedule", "scheduledAt": at});
        if let Err(e) = enqueue(st, &org, &id, &spec, Start { kind: WorkflowTriggerKind::Schedule, by: "Schedule".into(), run_as: publisher, trigger, inputs: Map::new() }).await {
            tracing::warn!(error = %e, workflow = id, "scheduled run not started");
        }
    }
    Ok(())
}

fn permits() -> &'static Arc<Semaphore> {
    static P: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
    P.get_or_init(|| Arc::new(Semaphore::new(CONCURRENT_RUNS)))
}

/// Claims due runs and executes each in its own task.
async fn claim_and_run(st: &AppState) -> Result<(), ApiError> {
    let free = permits().available_permits();
    if free == 0 {
        return Ok(());
    }
    let mut tx = st.postgres.begin().await?;
    let ids: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM workflow_runs WHERE status IN ('queued', 'running', 'waiting') AND next_at <= now() AND (lease_until IS NULL OR lease_until < now()) \
         ORDER BY next_at LIMIT $1 FOR UPDATE SKIP LOCKED",
    )
    .bind(free as i64)
    .fetch_all(&mut *tx)
    .await?;
    let ids: Vec<i64> = ids.into_iter().map(|r| r.0).collect();
    if !ids.is_empty() {
        sqlx::query("UPDATE workflow_runs SET lease_until = now() + make_interval(secs => $2), status = 'running', started_at = COALESCE(started_at, now()) WHERE id = ANY($1)")
            .bind(&ids)
            .bind(LEASE_SECONDS)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for id in ids {
        let Ok(permit) = permits().clone().try_acquire_owned() else {
            // Released for another server (or the next tick).
            sqlx::query("UPDATE workflow_runs SET lease_until = NULL WHERE id = $1").bind(id).execute(&st.postgres).await?;
            continue;
        };
        let st = st.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(e) = execute(&st, id).await {
                tracing::error!(error = %e, run = id, "workflow run failed on the server");
                let _ = finish(&st, id, None, "failed", Some("The run failed on the server".into())).await;
            }
        });
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RunRow {
    org_id: String,
    workflow_id: String,
    workflow_name: String,
    run_as: Option<String>,
    trigger: Value,
    inputs: Value,
    spec: Value,
    state: Option<Value>,
    age_hours: f64,
}

/// The member a run acts as, while they may still run workflows.
async fn member(st: &AppState, org: &str, user: &str) -> Result<Option<(AuthContext, String)>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Member {
        user_id: String,
        account_id: String,
        name: String,
        email: String,
        status: String,
        roles: Vec<String>,
        org_active: bool,
    }
    let row: Option<Member> = sqlx::query_as(
        "SELECT u.id::text AS user_id, u.account_id::text AS account_id, u.name, u.email, u.status, \
         COALESCE((SELECT array_agg(role_id) FROM user_roles WHERE user_id = u.id), '{}') AS roles, (o.status = 'active') AS org_active \
         FROM users u JOIN orgs o ON o.id = u.org_id WHERE u.org_id = $1 AND u.id::text = $2",
    )
    .bind(org)
    .bind(user)
    .fetch_optional(&st.postgres)
    .await?;
    let Some(Member { user_id, account_id, name, email, status, roles, org_active }) = row else { return Ok(None) };
    let access = access_from_roles(org, &roles);
    if status != "active" || !org_active || access < Access::Standard {
        return Ok(None);
    }
    let ctx = AuthContext { user_id, org_id: org.to_string(), account_id, instance_admin: false, name, access, credential: Credential::AppKey, session_hash: None, ip: None };
    Ok(Some((ctx, email)))
}

async fn finish(st: &AppState, run: i64, state: Option<&RunState>, status: &str, error: Option<String>) -> Result<(), ApiError> {
    let state = state.map(json_of).transpose()?;
    sqlx::query("UPDATE workflow_runs SET status = $2, error = $3, state = COALESCE($4, state), finished_at = now(), lease_until = NULL WHERE id = $1 AND status IN ('queued', 'running', 'waiting')")
        .bind(run)
        .bind(status)
        .bind(error.map(|e| e.chars().take(2000).collect::<String>()))
        .bind(state)
        .execute(&st.postgres)
        .await?;
    Ok(())
}

/// Saves the state and renews the lease; false when the run was canceled meanwhile.
async fn checkpoint(st: &AppState, run: i64, state: &RunState) -> Result<bool, ApiError> {
    let r = sqlx::query("UPDATE workflow_runs SET state = $2, lease_until = now() + make_interval(secs => $3) WHERE id = $1 AND status = 'running'")
        .bind(run)
        .bind(json_of(state)?)
        .bind(LEASE_SECONDS)
        .execute(&st.postgres)
        .await?;
    Ok(r.rows_affected() == 1)
}

struct StepRecord<'a> {
    org: &'a str,
    run: i64,
    step_id: &'a str,
    kind: WorkflowStepKind,
    name: &'a str,
    iteration: Option<i32>,
    ok: bool,
    inputs: Value,
    output: Value,
    error: Option<String>,
    attempts: i32,
    duration_ms: f64,
}

async fn record(st: &AppState, r: StepRecord<'_>) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO workflow_run_steps (org_id, run_id, step_id, kind, name, iteration, status, inputs, output, error, attempts, started_at, duration_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now() - make_interval(secs => $12 / 1000.0), $12)",
    )
    .bind(r.org)
    .bind(r.run)
    .bind(r.step_id)
    .bind(r.kind.as_str())
    .bind(r.name)
    .bind(r.iteration)
    .bind(if r.ok { "succeeded" } else { "failed" })
    .bind(r.inputs)
    .bind(r.output)
    .bind(r.error.map(|e| e.chars().take(2000).collect::<String>()))
    .bind(r.attempts)
    .bind(r.duration_ms)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

async fn execute(st: &AppState, run: i64) -> Result<(), ApiError> {
    let row: RunRow = sqlx::query_as(
        "SELECT r.org_id, r.workflow_id::text, w.name AS workflow_name, r.run_as::text, r.trigger, r.inputs, r.spec, r.state, \
         (extract(epoch FROM now() - r.created_at) / 3600.0)::float8 AS age_hours \
         FROM workflow_runs r JOIN workflows w ON w.org_id = r.org_id AND w.id = r.workflow_id WHERE r.id = $1",
    )
    .bind(run)
    .fetch_one(&st.postgres)
    .await?;
    let org = row.org_id.clone();
    if row.age_hours > MAX_RUN_HOURS as f64 {
        return finish(st, run, None, "failed", Some(format!("The run took longer than {MAX_RUN_HOURS} hours"))).await;
    }
    let spec = spec_of(&row.spec)?;
    let Some((ctx, email)) = (match &row.run_as {
        Some(u) => member(st, &org, u).await?,
        None => None,
    }) else {
        return finish(st, run, None, "failed", Some("The member this run acts as (who published the workflow) can no longer run workflows; publish it again".into())).await;
    };
    let wf: Option<(Option<String>,)> = sqlx::query_as("SELECT secrets_enc FROM workflows WHERE org_id = $1 AND id::text = $2").bind(&org).bind(&row.workflow_id).fetch_optional(&st.postgres).await?;
    let secrets: BTreeMap<String, String> = match wf.and_then(|w| w.0) {
        Some(enc) => match st.secrets.decrypt(&enc).ok().and_then(|p| serde_json::from_str(&p).ok()) {
            Some(s) => s,
            None => return finish(st, run, None, "failed", Some("The workflow's secrets can't be decrypted (was OC_SECRET_KEY changed?); enter them again".into())).await,
        },
        None => BTreeMap::new(),
    };
    let inputs = row.inputs.as_object().cloned().unwrap_or_default();
    let depth = row.trigger.get("chain").and_then(Value::as_u64).unwrap_or(0) as u8;
    let base = json!({
        "Trigger": row.trigger,
        "inputs": inputs,
        "Workflow": {"id": row.workflow_id, "name": row.workflow_name, "runId": run, "url": format!("{}/automation/workflows/{}?run={run}", st.config.public_url, row.workflow_id)},
        "user": {"name": ctx.name, "email": email},
    });
    let mut state: RunState = match &row.state {
        Some(s) => serde_json::from_value(s.clone()).map_err(|e| ApiError::Internal(anyhow::anyhow!("stored run state unreadable: {e}")))?,
        None => engine::start(),
    };
    loop {
        let mut decisions = Vec::new();
        let next = engine::advance(&spec, &mut state, &base, &mut decisions);
        for d in decisions {
            record(
                st,
                StepRecord {
                    org: &org,
                    run,
                    step_id: &d.step_id,
                    kind: d.kind,
                    name: &d.name,
                    iteration: d.iteration,
                    ok: true,
                    inputs: json!({}),
                    output: d.output,
                    error: None,
                    attempts: 1,
                    duration_ms: 0.0,
                },
            )
            .await?;
        }
        let (step, iteration) = match next {
            Next::Done => return finish(st, run, Some(&state), "succeeded", None).await,
            Next::Failed(e) => return finish(st, run, Some(&state), "failed", Some(e)).await,
            Next::Execute(step, iteration) => (step, iteration),
        };
        let vars = engine::vars(&state, &base);
        let rc = RenderContext { inputs: &inputs, user_email: &email, user_name: &ctx.name, secrets: &secrets, steps: Vec::new(), vars: Some(&vars) };
        if step.kind == WorkflowStepKind::Wait {
            let seconds = steps::render_text(step.fields.get("seconds").map(String::as_str).unwrap_or(""), &rc, true).and_then(|s| engine::wait_seconds(&s));
            let seconds = match seconds {
                Ok(s) => s,
                Err(e) => {
                    let e = format!("Step {}: {e}", step.id);
                    record(
                        st,
                        StepRecord {
                            org: &org,
                            run,
                            step_id: &step.id,
                            kind: step.kind,
                            name: &step.name,
                            iteration,
                            ok: false,
                            inputs: json!({}),
                            output: json!({}),
                            error: Some(e.clone()),
                            attempts: 1,
                            duration_ms: 0.0,
                        },
                    )
                    .await?;
                    return finish(st, run, Some(&state), "failed", Some(e)).await;
                }
            };
            let output = json!({"seconds": seconds, "until": now_ms() + seconds * 1000});
            record(
                st,
                StepRecord {
                    org: &org,
                    run,
                    step_id: &step.id,
                    kind: step.kind,
                    name: &step.name,
                    iteration,
                    ok: true,
                    inputs: json!({"seconds": seconds}),
                    output: output.clone(),
                    error: None,
                    attempts: 1,
                    duration_ms: 0.0,
                },
            )
            .await?;
            engine::complete(&mut state, &step.id, true, output, None);
            let parked = sqlx::query("UPDATE workflow_runs SET state = $2, status = 'waiting', next_at = now() + make_interval(secs => $3), lease_until = NULL WHERE id = $1 AND status = 'running'")
                .bind(run)
                .bind(json_of(&state)?)
                .bind(seconds as f64)
                .execute(&st.postgres)
                .await?;
            if parked.rows_affected() == 0 {
                tracing::info!(run, "workflow run canceled during a wait step");
            }
            return Ok(());
        }
        let started = Instant::now();
        let mut attempts = 0;
        let outcome = loop {
            attempts += 1;
            let attempt = CHAIN.scope(depth + 1, tokio::time::timeout(STEP_TIMEOUT, run_step(st, &ctx, &row.workflow_name, &base, &step, &rc))).await;
            let o = attempt.unwrap_or_else(|_| StepOutcome::failed(json!({}), format!("The step took longer than {} seconds", STEP_TIMEOUT.as_secs())));
            if o.ok || attempts > step.retries as i32 {
                break o;
            }
            tokio::time::sleep(RETRY_DELAY).await;
        };
        let output = cap_output(redact_value(&outcome.output, &secrets));
        let error = redact_opt(outcome.error, &secrets);
        record(
            st,
            StepRecord {
                org: &org,
                run,
                step_id: &step.id,
                kind: step.kind,
                name: &step.name,
                iteration,
                ok: outcome.ok,
                inputs: cap_output(redact_value(&outcome.inputs, &secrets)),
                output: output.clone(),
                error: error.clone(),
                attempts,
                duration_ms: started.elapsed().as_secs_f64() * 1000.0,
            },
        )
        .await?;
        engine::complete(&mut state, &step.id, outcome.ok, output, error.clone());
        if !outcome.ok && !step.continue_on_error {
            return finish(st, run, Some(&state), "failed", Some(format!("Step {}: {}", step.id, error.unwrap_or_else(|| "failed".into())))).await;
        }
        if !checkpoint(st, run, &state).await? {
            // Canceled while the step ran.
            return Ok(());
        }
    }
}

struct StepOutcome {
    ok: bool,
    inputs: Value,
    output: Value,
    error: Option<String>,
}

impl StepOutcome {
    fn failed(inputs: Value, e: String) -> Self {
        StepOutcome { ok: false, inputs, output: json!({}), error: Some(e) }
    }
}

fn field<'a>(step: &'a WorkflowStep, name: &str) -> &'a str {
    step.fields.get(name).map(String::as_str).unwrap_or("")
}

async fn run_step(st: &AppState, ctx: &AuthContext, workflow_name: &str, base: &Value, step: &WorkflowStep, rc: &RenderContext<'_>) -> StepOutcome {
    if let Some(kind) = step.kind.action_kind() {
        let action = ActionStep { kind, requests: step.requests.clone(), fields: step.fields.clone() };
        let rendered: Value = if kind == steps::ActionStepKind::Http {
            Value::Array(step.requests.iter().map(|r| json!({"method": r.method, "url": steps::render_url(&r.url, rc).unwrap_or_else(|e| e)})).collect())
        } else {
            Value::Object(step.fields.iter().map(|(k, v)| (k.clone(), Value::String(steps::render_text(v, rc, false).unwrap_or_else(|e| e)))).collect())
        };
        let o = if kind == steps::ActionStepKind::Http {
            let mut rc = RenderContext { inputs: rc.inputs, user_email: rc.user_email, user_name: rc.user_name, secrets: rc.secrets, steps: Vec::new(), vars: rc.vars };
            run_http(st, &action, &mut rc).await
        } else {
            run_internal(st, ctx, &action, rc).await
        };
        let mut output = o.output;
        if output.is_null() {
            output = json!({"message": o.response, "link": o.link});
        } else if let Some(m) = output.as_object_mut() {
            m.entry("link").or_insert(Value::String(o.link.clone()));
        }
        let inputs = if kind == steps::ActionStepKind::Http { json!({"requests": rendered}) } else { rendered };
        return StepOutcome { ok: o.ok, inputs, output, error: o.error };
    }
    let render = |name: &str| steps::render_text(field(step, name), rc, false).map(|s| s.trim().to_string());
    let result: Result<(Value, Value), (Value, String)> = async {
        match step.kind {
            WorkflowStepKind::Notify => {
                let (who, title, message) = (render("recipients").map_err(|e| (json!({}), e))?, render("title").map_err(|e| (json!({}), e))?, render("message").map_err(|e| (json!({}), e))?);
                let recipients = recipient_list(&who);
                let inputs = json!({"recipients": recipients, "title": title, "message": message});
                if recipients.is_empty() || recipients.len() > MAX_RECIPIENTS {
                    return Err((inputs, format!("Name 1–{MAX_RECIPIENTS} recipients (@slack-…, @webhook-… or member emails)")));
                }
                let title = if title.is_empty() { format!("[Workflow] {workflow_name}") } else { title };
                let link = base["Workflow"]["url"].as_str().unwrap_or_default().to_string();
                let out = super::notifications::Outgoing {
                    org: ctx.org_id.clone(),
                    title: title.clone(),
                    email_body: format!("{title}\n\n{message}\n\nWorkflow run: {link}\n"),
                    slack_text: format!("*{title}*\n{message}\n<{link}|View workflow run>"),
                    webhook: json!({"event": "workflow.notify", "title": title, "message": message, "workflow": base["Workflow"], "timestamp": now_ms()}),
                };
                let mut sent = Vec::new();
                let mut failed = Vec::new();
                for r in &recipients {
                    match super::notifications::send(st, r, &out).await {
                        super::notifications::Outcome::Sent => sent.push(r.clone()),
                        super::notifications::Outcome::Unknown(e) | super::notifications::Outcome::Failed(e) => failed.push(json!({"recipient": r, "error": e})),
                    }
                }
                let output = json!({"sent": sent, "failed": failed});
                if failed.is_empty() {
                    Ok((inputs, output))
                } else {
                    let why: Vec<String> = failed.iter().map(|f| format!("@{}: {}", f["recipient"].as_str().unwrap_or(""), f["error"].as_str().unwrap_or(""))).collect();
                    Err((inputs, why.join("; ")))
                }
            }
            WorkflowStepKind::MetricQuery => {
                let query = render("query").map_err(|e| (json!({}), e))?;
                let minutes = render("minutes").and_then(|m| engine::window_minutes(&m)).map_err(|e| (json!({"query": query}), e))?;
                let aggregation = render("aggregation").map_err(|e| (json!({}), e))?;
                let aggregation = if aggregation.is_empty() { "avg".to_string() } else { aggregation };
                let inputs = json!({"query": query, "minutes": minutes, "aggregation": aggregation});
                let agg: super::metrics::TimeAggregator =
                    serde_json::from_value(Value::String(aggregation.clone())).map_err(|_| (inputs.clone(), format!("{aggregation} isn't an aggregation (avg, sum, min, max, last)")))?;
                let to = now_ms();
                let range = super::logs::TimeRange { from: to - minutes * 60_000, to };
                let groups = super::metrics::grouped_values(st, &ctx.org_id, &query, range, agg).await.map_err(|e| (inputs.clone(), api_message(e)))?;
                let value = groups.first().map(|g| g.value);
                let list: Vec<Value> = groups.iter().take(50).map(|g| json!({"group": g.group, "tags": g.tags, "value": g.value})).collect();
                Ok((inputs, json!({"value": value, "groups": list})))
            }
            WorkflowStepKind::LogCount => {
                let query = render("query").map_err(|e| (json!({}), e))?;
                let minutes = render("minutes").and_then(|m| engine::window_minutes(&m)).map_err(|e| (json!({"query": query}), e))?;
                let group_by = render("group_by").map_err(|e| (json!({}), e))?;
                let inputs = json!({"query": query, "minutes": minutes, "groupBy": group_by});
                let access = crate::log_access::for_user(st, ctx).await.map_err(|e| (inputs.clone(), api_message(e)))?;
                let to = now_ms();
                let range = super::logs::TimeRange { from: to - minutes * 60_000, to };
                let by: Vec<String> = if group_by.is_empty() { vec![] } else { vec![group_by.clone()] };
                let groups = super::logs::log_counts(st, &ctx.org_id, &access, &query, &by, range, minutes * 60_000).await.map_err(|e| (inputs.clone(), api_message(e)))?;
                let mut counted: Vec<(String, f64)> = groups.iter().map(|g| (g.tags.values().cloned().collect::<Vec<_>>().join(", "), g.points.iter().map(|p| p.1).sum())).collect();
                counted.sort_by(|a, b| b.1.total_cmp(&a.1));
                let total: f64 = counted.iter().map(|c| c.1).sum();
                let list: Vec<Value> = counted.iter().take(50).map(|(g, n)| json!({"group": if g.is_empty() { "*" } else { g }, "count": n})).collect();
                Ok((inputs, json!({"count": total, "groups": list})))
            }
            _ => Err((json!({}), "This step can't run here".into())),
        }
    }
    .await;
    match result {
        Ok((inputs, output)) => StepOutcome { ok: true, inputs, output, error: None },
        Err((inputs, e)) => StepOutcome::failed(inputs, e),
    }
}

/// The message of an API error, storage errors kept in the server log.
fn api_message(e: ApiError) -> String {
    match e {
        ApiError::BadRequest(m) | ApiError::NotFound(m) | ApiError::Forbidden(m) | ApiError::Unauthorized(m) | ApiError::TooManyRequests(m) | ApiError::Conflict(m) => m,
        other => {
            tracing::error!(error = %other, "workflow step failed");
            "The step failed on the server".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> WorkflowInput {
        WorkflowInput {
            name: name.into(),
            handle: String::new(),
            description: String::new(),
            tags: vec!["team:checkout".into(), "team:checkout".into()],
            owner_team: "@checkout".into(),
            service: "order-service".into(),
            blueprint: String::new(),
            spec: WorkflowSpec::default(),
            secrets: BTreeMap::new(),
        }
    }

    #[test]
    fn handles_come_from_names() {
        assert_eq!(handle_from("Restart order-service when error rate alerts"), "restart-order-service-when-error-rate-alerts");
        assert_eq!(handle_from("  Daily top errors → #checkout!  "), "daily-top-errors-checkout");
        assert_eq!(handle_from("🚀"), "workflow");
        assert_eq!(handle_from(&"a".repeat(80)).len(), 60);
    }

    #[test]
    fn workflows_are_checked() {
        let w = check_workflow(&input(" Page payments "), None, &BTreeMap::new()).unwrap();
        assert_eq!((w.name.as_str(), w.handle.as_str(), w.owner_team.as_str()), ("Page payments", "page-payments", "checkout"));
        assert_eq!(w.tags, vec!["team:checkout"]);
        // Renaming keeps the handle; an explicit one replaces it.
        assert_eq!(check_workflow(&input("Renamed"), Some("page-payments"), &BTreeMap::new()).unwrap().handle, "page-payments");
        let explicit = WorkflowInput { handle: "@workflow-billing_burn".into(), ..input("x") };
        assert_eq!(check_workflow(&explicit, None, &BTreeMap::new()).unwrap().handle, "billing_burn");
        let bad = WorkflowInput { handle: "Has Spaces".into(), ..input("x") };
        assert!(check_workflow(&bad, None, &BTreeMap::new()).unwrap_err().contains("handle"));
        assert!(check_workflow(&input(""), None, &BTreeMap::new()).unwrap_err().contains("name"));
        let team = WorkflowInput { owner_team: "a team".into(), ..input("x") };
        assert!(check_workflow(&team, None, &BTreeMap::new()).unwrap_err().contains("team"));
    }

    #[test]
    fn recipients_and_outputs() {
        assert_eq!(recipient_list("@slack-checkout, oncall@example.com @webhook-ops,@slack-checkout"), vec!["slack-checkout", "oncall@example.com", "webhook-ops"]);
        assert!(recipient_list(" , ").is_empty());
        let small = json!({"a": 1});
        assert_eq!(cap_output(small.clone()), small);
        let big = json!({"body": "x".repeat(MAX_OUTPUT_CHARS + 1)});
        assert_eq!(cap_output(big)["truncated"], json!(true));
        let secrets = BTreeMap::from([("token".to_string(), "s3cr3t".to_string())]);
        assert_eq!(redact_value(&json!({"echo": "Bearer s3cr3t"}), &secrets), json!({"echo": "Bearer [REDACTED]"}));
        assert!(constant_time_eq(b"abc", b"abc") && !constant_time_eq(b"abc", b"abd") && !constant_time_eq(b"abc", b"ab"));
    }
}
