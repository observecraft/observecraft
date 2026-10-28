//! Developer Portal → Self-Service Actions: forms that run one step for the developer who fills them.
//!
//! - **Actions** have a name, category, owner team, form inputs (text, select, number, boolean) and
//!   one step: HTTP requests sent in order (each templated, see [`super::portal_action_steps`]) or an
//!   ObserveCraft operation (declare an incident, page an on-call team, add a note to an incident,
//!   run a synthetic test, start a workflow that has an app trigger). Standard users and admins create and edit them; blueprints in the UI
//!   pre-fill them.
//! - **Credentials** for HTTP requests are stored per action, sealed with the server's secret key
//!   (`OC_SECRET_KEY`), referred to as `{{secrets.<name>}}` and never returned: the API lists their
//!   names only, and run records have them redacted.
//! - **Runs.** Standard users and admins run actions, or only the members of the teams an action is
//!   restricted to (admins always may). Requests go out through the synthetic tests' client, so
//!   every target is resolved and checked against private and internal addresses (unless the
//!   operator allows them, `OC_SYNTHETICS_ALLOW_PRIVATE`), redirects aren't followed, each request
//!   is bounded to 30 s and its body to 1 MiB. A request succeeds on a 2xx status. Each run is kept
//!   with its status, response code, duration, a redacted excerpt of the response and who ran it,
//!   and is audited.

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

use super::portal_action_steps::{self as steps, ActionInput, ActionStep, ActionStepKind, RenderContext};
use super::synthetics_exec::{self as exec, SyntheticHeader, SyntheticRequest, SyntheticTestType};
use crate::auth::{Access, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CurrentUser, OrgId};

const MAX_ACTIONS: i64 = 500;
const MAX_SECRETS: usize = 10;
const MAX_SECRET_CHARS: usize = 4096;
const MAX_TEAMS: usize = 50;
const EXCERPT_CHARS: usize = 2000;
const REQUEST_TIMEOUT_SECONDS: u32 = 30;
const RUNS_LISTED: i64 = 100;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PortalActionCategory {
    Scaffolding,
    ReleaseManagement,
    InfrastructureProvisioning,
    IncidentManagement,
    InfrastructureManagement,
    Other,
}

impl PortalActionCategory {
    fn as_str(self) -> &'static str {
        match self {
            PortalActionCategory::Scaffolding => "scaffolding",
            PortalActionCategory::ReleaseManagement => "release_management",
            PortalActionCategory::InfrastructureProvisioning => "infrastructure_provisioning",
            PortalActionCategory::IncidentManagement => "incident_management",
            PortalActionCategory::InfrastructureManagement => "infrastructure_management",
            PortalActionCategory::Other => "other",
        }
    }

    fn parse(s: &str) -> Self {
        serde_json::from_value(Value::String(s.to_string())).unwrap_or(PortalActionCategory::Other)
    }
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PortalActionInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub category: PortalActionCategory,
    /// Blueprint the action started from; empty when built from scratch.
    #[serde(default)]
    pub blueprint: String,
    /// Team handle; empty for none.
    #[serde(default)]
    pub owner_team: String,
    #[serde(default)]
    pub inputs: Vec<ActionInput>,
    pub step: ActionStep,
    /// Credentials by name: a value sets one, null removes it, names left out keep their value.
    #[serde(default)]
    pub secrets: BTreeMap<String, Option<String>>,
    /// Team handles whose members may run it; empty for every standard user and admin.
    #[serde(default)]
    pub allowed_teams: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PortalAction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: PortalActionCategory,
    pub blueprint: String,
    pub owner_team: String,
    pub inputs: Vec<ActionInput>,
    pub step: ActionStep,
    /// Names of the stored credentials (values are never returned).
    pub secret_names: Vec<String>,
    pub allowed_teams: Vec<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub runs: i64,
    /// succeeded | failed, for the latest run.
    pub last_run_status: Option<String>,
    pub last_run_at: Option<i64>,
    /// The signed-in user may run it.
    pub can_run: bool,
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
pub struct PortalActionRunInput {
    /// Value per input key.
    #[serde(default)]
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub inputs: Map<String, Value>,
}

#[derive(Serialize, ToSchema, Clone, Debug, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PortalActionRun {
    pub id: i64,
    pub action_id: String,
    pub user_name: String,
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub inputs: Value,
    /// succeeded | failed
    pub status: String,
    /// HTTP status of the last request sent.
    pub status_code: Option<i32>,
    pub duration_ms: f64,
    /// Start of the last response (or the step's outcome), credentials redacted.
    pub response_excerpt: String,
    pub error: Option<String>,
    /// What the run created (repository, pull request, issue, incident, page); may be empty.
    pub link: String,
    pub created_at: i64,
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

fn valid_handle(h: &str) -> bool {
    !h.is_empty() && h.len() <= 100 && h.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

fn teams(list: &[String]) -> Result<Vec<String>, String> {
    if list.len() > MAX_TEAMS {
        return Err(format!("At most {MAX_TEAMS} teams"));
    }
    let mut out: Vec<String> = Vec::new();
    for t in list {
        let t = t.trim().trim_start_matches('@');
        if !valid_handle(t) {
            return Err(format!("{t} isn't a team handle"));
        }
        if !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    Ok(out)
}

/// The credentials after applying `changes` to `current`.
pub fn merge_secrets(current: &BTreeMap<String, String>, changes: &BTreeMap<String, Option<String>>) -> Result<BTreeMap<String, String>, String> {
    let mut out = current.clone();
    for (name, value) in changes {
        match value {
            None => {
                out.remove(name);
            }
            Some(v) => {
                if name.is_empty() || name.len() > 40 || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                    return Err(format!("{name} isn't a secret name (a–z, 0–9 and _, up to 40)"));
                }
                let v = v.trim();
                if v.is_empty() || v.chars().count() > MAX_SECRET_CHARS {
                    return Err(format!("The secret {name} must be 1–{MAX_SECRET_CHARS} characters"));
                }
                out.insert(name.clone(), v.to_string());
            }
        }
    }
    if out.len() > MAX_SECRETS {
        return Err(format!("An action has at most {MAX_SECRETS} secrets"));
    }
    Ok(out)
}

#[derive(Debug)]
struct Checked {
    name: String,
    description: String,
    blueprint: String,
    owner_team: String,
    inputs: Vec<ActionInput>,
    step: ActionStep,
    secrets: BTreeMap<String, String>,
    allowed_teams: Vec<String>,
}

fn check_action(input: &PortalActionInput, current_secrets: &BTreeMap<String, String>) -> Result<Checked, String> {
    let name = text(&input.name, "The name", 200, true)?;
    let description = text(&input.description, "The description", 2000, false)?;
    let blueprint = text(&input.blueprint, "The blueprint", 100, false)?;
    let owner_team = input.owner_team.trim().trim_start_matches('@').to_string();
    if !owner_team.is_empty() && !valid_handle(&owner_team) {
        return Err(format!("{owner_team} isn't a team handle"));
    }
    let inputs = steps::check_inputs(&input.inputs)?;
    let secrets = merge_secrets(current_secrets, &input.secrets)?;
    if input.step.kind != ActionStepKind::Http && !secrets.is_empty() {
        return Err("Secrets are only used by HTTP requests".into());
    }
    let names: HashSet<String> = secrets.keys().cloned().collect();
    let step = steps::check_step(&input.step, &inputs, &names)?;
    let allowed_teams = teams(&input.allowed_teams)?;
    Ok(Checked { name, description, blueprint, owner_team, inputs, step, secrets, allowed_teams })
}

/// Comma-separated values of a rendered field.
fn list(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().trim_start_matches('@').to_string()).filter(|x| !x.is_empty()).collect()
}

/// An incident number written as `12`, `#12` or `IR-12`.
fn incident_number(s: &str) -> Option<i32> {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    let t = t.strip_prefix("IR-").or_else(|| t.strip_prefix("ir-")).unwrap_or(t);
    t.parse().ok().filter(|n| *n > 0)
}

/// The first `max` characters of a text.
fn excerpt(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

// ================================================================ storage

fn valid_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn not_found() -> ApiError {
    ApiError::NotFound("Action not found".into())
}

#[derive(sqlx::FromRow)]
struct ActionRow {
    id: String,
    name: String,
    description: String,
    category: String,
    blueprint: String,
    owner_team: String,
    inputs: Value,
    step: Value,
    secrets_enc: Option<String>,
    allowed_teams: Vec<String>,
    created_by: String,
    created_at: i64,
    updated_at: i64,
    runs: i64,
    last_run_status: Option<String>,
    last_run_at: Option<i64>,
}

impl ActionRow {
    fn inputs(&self) -> Vec<ActionInput> {
        serde_json::from_value(self.inputs.clone()).unwrap_or_default()
    }

    fn step(&self) -> Result<ActionStep, ApiError> {
        serde_json::from_value(self.step.clone()).map_err(|e| ApiError::Internal(anyhow::anyhow!("stored action step unreadable: {e}")))
    }

    fn secrets(&self, st: &AppState) -> Result<BTreeMap<String, String>, ApiError> {
        let Some(enc) = &self.secrets_enc else { return Ok(BTreeMap::new()) };
        let plain = st.secrets.decrypt(enc).map_err(|_| ApiError::BadRequest("The action's secrets can't be decrypted (was OC_SECRET_KEY changed?); enter them again".into()))?;
        serde_json::from_str(&plain).map_err(|e| ApiError::Internal(anyhow::anyhow!("stored action secrets unreadable: {e}")))
    }

    fn view(&self, st: &AppState, may_run: bool) -> Result<PortalAction, ApiError> {
        Ok(PortalAction {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            category: PortalActionCategory::parse(&self.category),
            blueprint: self.blueprint.clone(),
            owner_team: self.owner_team.clone(),
            inputs: self.inputs(),
            step: self.step()?,
            secret_names: self.secrets(st).map(|s| s.into_keys().collect()).unwrap_or_default(),
            allowed_teams: self.allowed_teams.clone(),
            created_by: self.created_by.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            runs: self.runs,
            last_run_status: self.last_run_status.clone(),
            last_run_at: self.last_run_at,
            can_run: may_run,
        })
    }
}

async fn action_rows(st: &AppState, org: &str, id: Option<&str>) -> Result<Vec<ActionRow>, ApiError> {
    let rows = sqlx::query_as(
        "SELECT a.id::text, a.name, a.description, a.category, a.blueprint, a.owner_team, a.inputs, a.step, a.secrets_enc, a.allowed_teams, \
         COALESCE(u.name, '') AS created_by, (extract(epoch FROM a.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM a.updated_at) * 1000)::bigint AS updated_at, \
         (SELECT count(*) FROM portal_action_runs r WHERE r.org_id = a.org_id AND r.action_id = a.id) AS runs, \
         (SELECT r.status FROM portal_action_runs r WHERE r.org_id = a.org_id AND r.action_id = a.id ORDER BY r.created_at DESC, r.id DESC LIMIT 1) AS last_run_status, \
         (SELECT (extract(epoch FROM max(r.created_at)) * 1000)::bigint FROM portal_action_runs r WHERE r.org_id = a.org_id AND r.action_id = a.id) AS last_run_at \
         FROM portal_actions a LEFT JOIN users u ON u.id = a.created_by \
         WHERE a.org_id = $1 AND ($2::text IS NULL OR a.id::text = $2) ORDER BY a.name",
    )
    .bind(org)
    .bind(id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows)
}

async fn action_row(st: &AppState, org: &str, id: &str) -> Result<ActionRow, ApiError> {
    if !valid_uuid(id) {
        return Err(not_found());
    }
    action_rows(st, org, Some(id)).await?.pop().ok_or_else(not_found)
}

async fn user_teams(st: &AppState, org: &str, user: &str) -> Result<Vec<String>, ApiError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT handle FROM team_members WHERE org_id = $1 AND user_id::text = $2").bind(org).bind(user).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Standard users and admins run actions; a restricted one only by its teams' members (and admins).
fn may_run(ctx: &AuthContext, allowed_teams: &[String], my_teams: &[String]) -> bool {
    ctx.access == Access::Admin || (ctx.access >= Access::Standard && (allowed_teams.is_empty() || allowed_teams.iter().any(|t| my_teams.contains(t))))
}

fn seal(st: &AppState, secrets: &BTreeMap<String, String>) -> Result<Option<String>, ApiError> {
    if secrets.is_empty() {
        return Ok(None);
    }
    Ok(Some(st.secrets.encrypt(&serde_json::to_string(secrets).map_err(anyhow::Error::from)?)))
}

// ================================================================ running

/// What one run produced.
#[derive(Default)]
pub(crate) struct Outcome {
    pub ok: bool,
    pub status_code: Option<i32>,
    pub response: String,
    pub error: Option<String>,
    pub link: String,
    /// Workflows: what later steps read (`Steps.<id>.output`).
    pub output: Value,
}

/// The message of an error from an internal step (storage errors stay in the server log).
fn step_error(e: ApiError) -> String {
    match e {
        ApiError::BadRequest(m) | ApiError::NotFound(m) | ApiError::Forbidden(m) | ApiError::Unauthorized(m) | ApiError::TooManyRequests(m) => m,
        other => {
            tracing::error!(error = %other, "self-service action step failed");
            "The step failed on the server".into()
        }
    }
}

pub(crate) async fn run_http(st: &AppState, step: &ActionStep, rc: &mut RenderContext<'_>) -> Outcome {
    let mut out = Outcome::default();
    for (i, r) in step.requests.iter().enumerate() {
        let which = if step.requests.len() > 1 { format!("Request {}: ", i + 1) } else { String::new() };
        let rendered = (|| -> Result<SyntheticRequest, String> {
            let url = steps::render_url(&r.url, rc)?;
            let headers = r.headers.iter().map(|h| Ok(SyntheticHeader { name: h.name.clone(), value: steps::render_text(&h.value, rc, true)? })).collect::<Result<Vec<_>, String>>()?;
            let body = steps::render_json(&r.body, rc)?;
            let body_type = if body.is_empty() { String::new() } else { "application/json".into() };
            Ok(SyntheticRequest { method: r.method.clone(), url, headers, body, body_type, timeout_seconds: REQUEST_TIMEOUT_SECONDS, follow_redirects: false, ..SyntheticRequest::default() })
        })();
        let req = match rendered {
            Ok(req) => req,
            Err(e) => {
                out.error = Some(format!("{which}{e}"));
                return out;
            }
        };
        let res = exec::run(SyntheticTestType::Http, &req, &[], st.config.synthetics_allow_private).await;
        out.status_code = res.status_code.map(i32::from);
        out.response = res.body.clone().unwrap_or_default();
        if let Some(e) = res.error {
            out.error = Some(format!("{which}{e}"));
            return out;
        }
        let code = res.status_code.unwrap_or(0);
        if !(200..300).contains(&code) {
            out.error = Some(format!("{which}{} {} answered {code}", req.method.to_ascii_uppercase(), host_of(&req.url)));
            return out;
        }
        let json: Value = serde_json::from_str(&out.response).unwrap_or(Value::Null);
        if let Some(link) = steps::result_link(&json) {
            out.link = link;
        }
        out.output = serde_json::json!({"statusCode": code, "body": if json.is_null() { Value::String(excerpt(&out.response, EXCERPT_CHARS)) } else { json.clone() }, "link": out.link});
        rc.steps.push(json);
    }
    out.ok = true;
    out
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

pub(crate) async fn run_internal(st: &AppState, ctx: &AuthContext, step: &ActionStep, rc: &RenderContext<'_>) -> Outcome {
    let mut out = Outcome::default();
    let field = |name: &str| steps::render_text(step.fields.get(name).map(String::as_str).unwrap_or(""), rc, false).map(|s| s.trim().to_string());
    let result: Result<(String, String, Value), String> = async {
        match step.kind {
            ActionStepKind::CreateIncident => {
                let severity = field("severity")?;
                let req = super::incidents::DeclareRequest {
                    title: field("title")?,
                    severity: if severity.is_empty() { "SEV-3".into() } else { severity },
                    incident_type: "Production Incident".into(),
                    detection_method: None,
                    summary: field("summary")?,
                    commander_id: None,
                    services: list(&field("services")?),
                    teams: list(&field("teams")?),
                    customer_impact: false,
                    test: false,
                };
                let Json(detail) =
                    super::incidents::declare(State(st.clone()), Writer(ctx.clone()), OrgId(ctx.org_id.clone()), CurrentUser(ctx.user_id.clone()), Json(req)).await.map_err(step_error)?;
                let id = detail.incident.id;
                let output = serde_json::json!({"incident": id, "title": detail.incident.title, "severity": detail.incident.severity});
                Ok((format!("Declared incident IR-{id}: {}", detail.incident.title), format!("/incidents/{id}"), output))
            }
            ActionStepKind::PageOnCall => {
                let urgency = field("urgency")?;
                let req = super::on_call::CreateOnCallPage {
                    title: field("title")?,
                    description: field("description")?,
                    urgency: (!urgency.is_empty()).then_some(urgency),
                    team: Some(field("team")?.trim_start_matches('@').to_string()),
                    policy_id: None,
                };
                let Json(page) = super::on_call::create_page(State(st.clone()), Writer(ctx.clone()), Json(req)).await.map_err(step_error)?;
                let output = serde_json::json!({"page": page.id, "team": page.team, "title": page.title});
                Ok((format!("Paged {}: {}", page.team.as_deref().map(|t| format!("@{t}")).unwrap_or_default(), page.title), format!("/on-call/pages/{}", page.id), output))
            }
            ActionStepKind::IncidentComment => {
                let raw = field("incident")?;
                let id = incident_number(&raw).ok_or_else(|| format!("{raw} isn't an incident number"))?;
                let req = super::incidents::NoteRequest { body: field("body")? };
                super::incidents::add_note(State(st.clone()), Writer(ctx.clone()), OrgId(ctx.org_id.clone()), CurrentUser(ctx.user_id.clone()), Path(id), Json(req)).await.map_err(step_error)?;
                Ok((format!("Added a note to incident IR-{id}"), format!("/incidents/{id}"), serde_json::json!({"incident": id})))
            }
            ActionStepKind::SyntheticRun => {
                let test = field("test")?;
                let Json(result) = super::synthetics::run_now(State(st.clone()), Writer(ctx.clone()), Path(test.clone())).await.map_err(step_error)?;
                let link = format!("/synthetics/tests/{test}");
                if result.detail.passed {
                    Ok(("The synthetic test passed".into(), link, serde_json::json!({"test": test, "passed": true})))
                } else {
                    let why = result.detail.error.clone().unwrap_or_else(|| "an assertion failed".into());
                    Err(format!("The synthetic test failed: {why}"))
                }
            }
            ActionStepKind::RunWorkflow => {
                let id = field("workflow")?;
                let run = super::workflows::start_from_app(st, ctx, &id, "Self-service action", rc.inputs).await.map_err(step_error)?;
                Ok((format!("Started workflow run #{run}"), format!("/automation/workflows/{id}?run={run}"), serde_json::json!({"workflow": id, "run": run})))
            }
            ActionStepKind::Http => Err("Not an internal step".into()),
        }
    }
    .await;
    match result {
        Ok((response, link, output)) => {
            out.ok = true;
            out.response = response;
            out.link = link;
            out.output = output;
        }
        Err(e) => out.error = Some(e),
    }
    out
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/portal/actions", operation_id = "list_portal_actions", responses((status = 200, body = Vec<PortalAction>)), tag = "portal")]
pub async fn list_actions(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<PortalAction>> {
    let mine = user_teams(&st, &ctx.org_id, &ctx.user_id).await?;
    let rows = action_rows(&st, &ctx.org_id, None).await?;
    Ok(Json(rows.iter().map(|r| r.view(&st, may_run(&ctx, &r.allowed_teams, &mine))).collect::<Result<_, _>>()?))
}

#[utoipa::path(get, path = "/api/v1/portal/actions/{id}", operation_id = "get_portal_action", params(("id" = String, Path)),
    responses((status = 200, body = PortalAction), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn get_action(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<PortalAction> {
    let mine = user_teams(&st, &ctx.org_id, &ctx.user_id).await?;
    let row = action_row(&st, &ctx.org_id, &id).await?;
    Ok(Json(row.view(&st, may_run(&ctx, &row.allowed_teams, &mine))?))
}

#[utoipa::path(post, path = "/api/v1/portal/actions", operation_id = "create_portal_action", request_body = PortalActionInput,
    responses((status = 200, body = PortalAction), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn create_action(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<PortalActionInput>) -> ApiResult<PortalAction> {
    let a = check_action(&input, &BTreeMap::new()).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM portal_actions WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_ACTIONS {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_ACTIONS} actions")));
    }
    let created: Option<(String,)> = sqlx::query_as(
        "INSERT INTO portal_actions (org_id, name, description, category, blueprint, owner_team, inputs, step_kind, step, secrets_enc, allowed_teams, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, (SELECT id FROM users WHERE org_id = $1 AND id::text = $12)) \
         ON CONFLICT (org_id, name) DO NOTHING RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&a.name)
    .bind(&a.description)
    .bind(input.category.as_str())
    .bind(&a.blueprint)
    .bind(&a.owner_team)
    .bind(serde_json::to_value(&a.inputs).map_err(anyhow::Error::from)?)
    .bind(a.step.kind.as_str())
    .bind(serde_json::to_value(&a.step).map_err(anyhow::Error::from)?)
    .bind(seal(&st, &a.secrets)?)
    .bind(&a.allowed_teams)
    .bind(&ctx.user_id)
    .fetch_optional(&st.postgres)
    .await?;
    let Some((id,)) = created else {
        return Err(ApiError::BadRequest(format!("An action named {} exists already", a.name)));
    };
    ctx.audit(&st, "portal.action_created", &id, &format!("{} ({}, {} secrets)", a.name, a.step.kind.as_str(), a.secrets.len())).await;
    get_action(State(st), ctx, Path(id)).await
}

#[utoipa::path(put, path = "/api/v1/portal/actions/{id}", operation_id = "update_portal_action", params(("id" = String, Path)), request_body = PortalActionInput,
    responses((status = 200, body = PortalAction), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn update_action(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<PortalActionInput>) -> ApiResult<PortalAction> {
    let row = action_row(&st, &ctx.org_id, &id).await?;
    // Secrets that can't be decrypted any more are replaced by the ones sent now.
    let current = row.secrets(&st).unwrap_or_default();
    let a = check_action(&input, &current).map_err(ApiError::BadRequest)?;
    let updated = sqlx::query(
        "UPDATE portal_actions SET name = $3, description = $4, category = $5, blueprint = $6, owner_team = $7, inputs = $8, step_kind = $9, step = $10, \
         secrets_enc = $11, allowed_teams = $12, updated_at = now() \
         WHERE org_id = $1 AND id::text = $2 AND NOT EXISTS (SELECT 1 FROM portal_actions o WHERE o.org_id = $1 AND o.name = $3 AND o.id::text <> $2)",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&a.name)
    .bind(&a.description)
    .bind(input.category.as_str())
    .bind(&a.blueprint)
    .bind(&a.owner_team)
    .bind(serde_json::to_value(&a.inputs).map_err(anyhow::Error::from)?)
    .bind(a.step.kind.as_str())
    .bind(serde_json::to_value(&a.step).map_err(anyhow::Error::from)?)
    .bind(seal(&st, &a.secrets)?)
    .bind(&a.allowed_teams)
    .execute(&st.postgres)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::BadRequest(format!("An action named {} exists already", a.name)));
    }
    let changed: Vec<&String> = input.secrets.keys().collect();
    let detail = if changed.is_empty() { a.name.clone() } else { format!("{} (secrets changed: {})", a.name, changed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) };
    ctx.audit(&st, "portal.action_updated", &id, &detail).await;
    get_action(State(st), ctx, Path(id)).await
}

#[utoipa::path(delete, path = "/api/v1/portal/actions/{id}", operation_id = "delete_portal_action", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn delete_action(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row = action_row(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM portal_actions WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "portal.action_deleted", &id, &row.name).await;
    Ok(StatusCode::NO_CONTENT)
}

const RUN_COLUMNS: &str = "id, action_id::text, user_name, inputs, status, status_code, duration_ms, response_excerpt, error, link, (extract(epoch FROM created_at) * 1000)::bigint AS created_at";

/// Runs the action now with the given inputs, as the signed-in user.
#[utoipa::path(post, path = "/api/v1/portal/actions/{id}/runs", operation_id = "run_portal_action", params(("id" = String, Path)), request_body = PortalActionRunInput,
    responses((status = 200, body = PortalActionRun), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn run_action(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<PortalActionRunInput>) -> ApiResult<PortalActionRun> {
    let row = action_row(&st, &ctx.org_id, &id).await?;
    let mine = user_teams(&st, &ctx.org_id, &ctx.user_id).await?;
    if !may_run(&ctx, &row.allowed_teams, &mine) {
        return Err(ApiError::Forbidden(format!("Only members of {} can run this action", row.allowed_teams.iter().map(|t| format!("@{t}")).collect::<Vec<_>>().join(", "))));
    }
    let inputs = steps::check_run_inputs(&row.inputs(), &input.inputs).map_err(ApiError::BadRequest)?;
    let step = row.step()?;
    let secrets = row.secrets(&st)?;
    let (email,): (String,) = sqlx::query_as("SELECT email FROM users WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    let mut rc = RenderContext { inputs: &inputs, user_email: &email, user_name: &ctx.name, secrets: &secrets, steps: Vec::new(), vars: None };
    let started = Instant::now();
    let outcome = if step.kind == ActionStepKind::Http { run_http(&st, &step, &mut rc).await } else { run_internal(&st, &ctx, &step, &rc).await };
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = if outcome.ok { "succeeded" } else { "failed" };
    let response = excerpt(&steps::redact(&outcome.response, &secrets), EXCERPT_CHARS);
    let error = outcome.error.map(|e| excerpt(&steps::redact(&e, &secrets), EXCERPT_CHARS));
    let run: PortalActionRun = sqlx::query_as(&format!(
        "INSERT INTO portal_action_runs (org_id, action_id, user_id, user_name, inputs, status, status_code, duration_ms, response_excerpt, error, link) \
         VALUES ($1, $2::uuid, (SELECT id FROM users WHERE org_id = $1 AND id::text = $3), $4, $5, $6, $7, $8, $9, $10, $11) RETURNING {RUN_COLUMNS}"
    ))
    .bind(&ctx.org_id)
    .bind(&row.id)
    .bind(&ctx.user_id)
    .bind(&ctx.name)
    .bind(Value::Object(inputs.clone()))
    .bind(status)
    .bind(outcome.status_code)
    .bind(duration_ms)
    .bind(&response)
    .bind(&error)
    .bind(&outcome.link)
    .fetch_one(&st.postgres)
    .await?;
    let code = outcome.status_code.map(|c| format!(", {c}")).unwrap_or_default();
    ctx.audit(&st, "portal.action_run", &row.id, &format!("{}: {status}{code}", row.name)).await;
    Ok(Json(run))
}

#[utoipa::path(get, path = "/api/v1/portal/actions/{id}/runs", operation_id = "list_portal_action_runs", params(("id" = String, Path)),
    responses((status = 200, body = Vec<PortalActionRun>), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn list_runs(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Vec<PortalActionRun>> {
    let row = action_row(&st, &ctx.org_id, &id).await?;
    let runs = sqlx::query_as(&format!("SELECT {RUN_COLUMNS} FROM portal_action_runs WHERE org_id = $1 AND action_id::text = $2 ORDER BY created_at DESC, id DESC LIMIT $3"))
        .bind(&ctx.org_id)
        .bind(&row.id)
        .bind(RUNS_LISTED)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(runs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_merge_set_keep_and_remove() {
        let current = BTreeMap::from([("github_token".to_string(), "old".to_string()), ("jira_token".to_string(), "j".to_string())]);
        let changes = BTreeMap::from([("github_token".to_string(), Some(" new ".to_string())), ("jira_token".to_string(), None), ("aws_key".to_string(), Some("k".to_string()))]);
        let merged = merge_secrets(&current, &changes).unwrap();
        assert_eq!(merged, BTreeMap::from([("aws_key".to_string(), "k".to_string()), ("github_token".to_string(), "new".to_string())]));
        assert!(merge_secrets(&current, &BTreeMap::from([("Bad Name".to_string(), Some("x".to_string()))])).unwrap_err().contains("Bad Name"));
        assert!(merge_secrets(&current, &BTreeMap::from([("t".to_string(), Some(" ".to_string()))])).unwrap_err().contains("1–4096"));
        let many: BTreeMap<String, Option<String>> = (0..11).map(|i| (format!("s{i}"), Some("v".to_string()))).collect();
        assert!(merge_secrets(&BTreeMap::new(), &many).unwrap_err().contains("at most 10"));
    }

    #[test]
    fn actions_are_checked() {
        let step: ActionStep = serde_json::from_value(serde_json::json!({
            "kind": "http",
            "requests": [{"method": "POST", "url": "https://api.github.com/repos/{{inputs.owner}}/x/generate", "headers": [{"name": "Authorization", "value": "Bearer {{secrets.github_token}}"}], "body": ""}]
        }))
        .unwrap();
        let owner = ActionInput { key: "owner".into(), label: "Owner".into(), kind: steps::ActionInputKind::Text, required: true, options: vec![], default: String::new(), description: String::new() };
        let input = PortalActionInput {
            name: " Scaffold a service ".into(),
            description: String::new(),
            category: PortalActionCategory::Scaffolding,
            blueprint: "github_scaffold".into(),
            owner_team: "@platform".into(),
            inputs: vec![owner],
            step,
            secrets: BTreeMap::from([("github_token".to_string(), Some("ghp_x".to_string()))]),
            allowed_teams: vec!["platform".into(), "@platform".into()],
        };
        let ok = check_action(&input, &BTreeMap::new()).unwrap();
        assert_eq!(ok.name, "Scaffold a service");
        assert_eq!(ok.owner_team, "platform");
        assert_eq!(ok.allowed_teams, vec!["platform"]);
        // The secret the request uses must exist.
        let without = PortalActionInput { secrets: BTreeMap::new(), ..input.clone() };
        assert!(check_action(&without, &BTreeMap::new()).unwrap_err().contains("github_token"));
        assert!(check_action(&without, &BTreeMap::from([("github_token".to_string(), "kept".to_string())])).is_ok(), "stored secrets are kept");
        let internal: ActionStep = serde_json::from_value(serde_json::json!({"kind": "page_on_call", "fields": {"team": "checkout", "title": "Help"}})).unwrap();
        let with_secret = PortalActionInput { step: internal, ..input.clone() };
        assert!(check_action(&with_secret, &BTreeMap::new()).unwrap_err().contains("HTTP"));
    }

    #[test]
    fn incident_numbers_and_lists() {
        assert_eq!(incident_number("12"), Some(12));
        assert_eq!(incident_number(" #7 "), Some(7));
        assert_eq!(incident_number("IR-31"), Some(31));
        assert_eq!(incident_number("0"), None);
        assert_eq!(incident_number("seven"), None);
        assert_eq!(list(" checkout, @payments ,,"), vec!["checkout", "payments"]);
        assert_eq!(excerpt("abcdef", 3), "abc…");
        assert_eq!(excerpt("abc", 3), "abc");
    }

    #[test]
    fn who_may_run() {
        let ctx = |access| AuthContext {
            user_id: "u".into(),
            org_id: "o".into(),
            account_id: "a".into(),
            instance_admin: false,
            name: "Dana".into(),
            access,
            credential: crate::auth::Credential::Session,
            session_hash: None,
            ip: None,
        };
        let restricted = vec!["platform".to_string()];
        assert!(may_run(&ctx(Access::Standard), &[], &[]));
        assert!(!may_run(&ctx(Access::ReadOnly), &[], &[]));
        assert!(!may_run(&ctx(Access::Standard), &restricted, &["checkout".into()]));
        assert!(may_run(&ctx(Access::Standard), &restricted, &["platform".into()]));
        assert!(!may_run(&ctx(Access::ReadOnly), &restricted, &["platform".into()]));
        assert!(may_run(&ctx(Access::Admin), &restricted, &[]));
    }
}
