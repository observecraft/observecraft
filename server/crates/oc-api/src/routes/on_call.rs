//! On-Call (Incident Response → On-Call): schedules, escalation policies, team routing, pages and
//! each member's notification preferences.
//!
//! - **Schedules** rotate responders in layers, with overrides; [`super::on_call_schedule`] computes
//!   who is on call when (time zones and DST included).
//! - **Escalation policies** list steps (users, schedules or teams to notify, then wait N minutes
//!   for an acknowledgement); a team's pages follow the policy routed to it (Teams tab). A team
//!   without a policy is paged as a whole: every member is notified once.
//! - **Pages** come from the Pages tab, from an incident ("Page a team") or from a monitor whose
//!   message mentions `@oncall-<team>` (a recovery resolves the monitor's open pages). The
//!   escalation worker ([`run_escalations`]) claims due pages with `FOR UPDATE SKIP LOCKED`, so with
//!   several servers each step notifies once, and resolves pages whose policy's auto-resolve time
//!   has passed.
//! - **Notifications** go through the monitors' channels: email (the member's address), Slack and
//!   webhooks, chosen per urgency in each member's On-Call profile (email when unset). There is no
//!   phone, SMS or push delivery.
//!
//! Permissions: everyone in the org can read; schedules, overrides, policies, routing, new pages
//! and reassigning need the Standard or Admin role. Acknowledging or resolving a page is also open
//! to a read-only member the page notified, so whoever is paged can always answer it. Every
//! member edits their own profile. Configuration changes are written to the audit trail.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder};
use utoipa::ToSchema;

use super::notifications::{send, Notice, Outcome, Outgoing};
use super::on_call_escalation::{after_notifying, steps_problem, transition, OnCallPageStatus, OnCallStep, OnCallTarget, OnCallTargetKind, PageAction, Progress};
use super::on_call_schedule::{final_schedule, layer_problem, layer_shifts, on_call_at, parse_tz, OnCallFinalShift, OnCallLayer, OnCallShift, OverrideSpan};
use crate::auth::{Access, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_NAME: usize = 200;
const MAX_TAGS: usize = 30;
const MAX_LAYERS: usize = 10;
const MAX_LAYER_MEMBERS: usize = 50;
const MAX_RESTRICTIONS: usize = 14;
const MAX_SCHEDULES: i64 = 500;
const MAX_POLICIES: i64 = 500;
const MAX_TITLE: usize = 300;
const MAX_DESCRIPTION: usize = 10_000;
/// Calendar and timeline requests span at most this long.
const MAX_RANGE_MS: i64 = 93 * 86_400_000;
const MAX_ANALYTICS_MS: i64 = 92 * 86_400_000;
/// Pages notified per worker tick.
const ESCALATION_BATCH: i64 = 20;
const SYSTEM_ACTOR: &str = "ObserveCraft";

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

fn text(s: &str, what: &str, max: usize) -> Result<String, ApiError> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(ApiError::BadRequest(format!("{what} must be 1–{max} characters")));
    }
    Ok(t.to_string())
}

fn clean_tags(tags: &[String]) -> Result<Vec<String>, ApiError> {
    if tags.len() > MAX_TAGS {
        return Err(ApiError::BadRequest(format!("At most {MAX_TAGS} tags")));
    }
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().to_lowercase();
        if t.is_empty() || t.len() > 200 {
            return Err(ApiError::BadRequest("Tags are 1–200 characters".into()));
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

// ================================================================ shared

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallUser {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct OnCallRef {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, ToSchema, Clone, Copy)]
pub struct OnCallRange {
    /// Unix ms.
    pub from: i64,
    pub to: i64,
}

impl OnCallRange {
    fn check(&self, max: i64) -> Result<(), ApiError> {
        if self.to <= self.from || self.to - self.from > max {
            return Err(ApiError::BadRequest(format!("The range must be positive and at most {} days", max / 86_400_000)));
        }
        Ok(())
    }
}

/// Active members of the org among `ids`, by id.
async fn users_by_id(st: &AppState, org: &str, ids: &[String]) -> Result<HashMap<String, OnCallUser>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<OnCallUser> =
        sqlx::query_as("SELECT id::text AS id, name, email FROM users WHERE org_id = $1 AND status = 'active' AND id::text = ANY($2)").bind(org).bind(ids).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|u| (u.id.clone(), u)).collect())
}

fn pick_users(by_id: &HashMap<String, OnCallUser>, ids: &[String]) -> Vec<OnCallUser> {
    ids.iter().filter_map(|id| by_id.get(id).cloned()).collect()
}

async fn check_users(st: &AppState, org: &str, ids: &[String]) -> Result<(), ApiError> {
    let unique: Vec<String> = ids.iter().cloned().collect::<HashSet<_>>().into_iter().collect();
    let found = users_by_id(st, org, &unique).await?;
    if let Some(missing) = unique.iter().find(|id| !found.contains_key(*id)) {
        return Err(ApiError::BadRequest(format!("Unknown or inactive user {missing}")));
    }
    Ok(())
}

async fn check_team(st: &AppState, org: &str, team: &str) -> Result<(), ApiError> {
    let found: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM teams WHERE org_id = $1 AND handle = $2").bind(org).bind(team).fetch_optional(&st.postgres).await?;
    found.map(|_| ()).ok_or_else(|| ApiError::BadRequest(format!("Unknown team @{team}")))
}

async fn optional_team(st: &AppState, org: &str, team: &Option<String>) -> Result<Option<String>, ApiError> {
    match team.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) => {
            check_team(st, org, t).await?;
            Ok(Some(t.to_string()))
        }
    }
}

// ================================================================ schedules

#[derive(Serialize, ToSchema, Clone, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallOverride {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub start: i64,
    pub end: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallSchedule {
    pub id: String,
    pub name: String,
    /// IANA time zone the layers' times are in.
    pub time_zone: String,
    pub team: Option<String>,
    pub tags: Vec<String>,
    pub layers: Vec<OnCallLayer>,
    pub max_concurrent: Option<i32>,
    /// Overrides that haven't ended more than 31 days ago.
    pub overrides: Vec<OnCallOverride>,
    pub on_call_now: Vec<OnCallUser>,
    /// The people of the layers (names for the editor).
    pub members: Vec<OnCallUser>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OnCallScheduleInput {
    pub name: String,
    pub time_zone: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub layers: Vec<OnCallLayer>,
    #[serde(default)]
    pub max_concurrent: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct ScheduleRow {
    id: String,
    name: String,
    time_zone: String,
    team_handle: Option<String>,
    tags: Vec<String>,
    layers: Value,
    max_concurrent: Option<i32>,
    created_at: i64,
    updated_at: i64,
}

impl ScheduleRow {
    fn layers(&self) -> Vec<OnCallLayer> {
        serde_json::from_value(self.layers.clone()).unwrap_or_default()
    }
    fn tz(&self) -> chrono_tz::Tz {
        parse_tz(&self.time_zone).unwrap_or(chrono_tz::UTC)
    }
    fn max(&self) -> Option<usize> {
        self.max_concurrent.map(|m| m.max(1) as usize)
    }
}

const SCHEDULE_SELECT: &str = "SELECT id::text AS id, name, time_zone, team_handle, tags, layers, max_concurrent, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at FROM on_call_schedules";

async fn schedule_rows(st: &AppState, org: &str) -> Result<Vec<ScheduleRow>, ApiError> {
    Ok(sqlx::query_as(&format!("{SCHEDULE_SELECT} WHERE org_id = $1 ORDER BY lower(name) LIMIT {MAX_SCHEDULES}")).bind(org).fetch_all(&st.postgres).await?)
}

async fn schedule_row(st: &AppState, org: &str, id: &str) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as(&format!("{SCHEDULE_SELECT} WHERE org_id = $1 AND id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Schedule not found".into()))
}

/// Overrides of the schedules ending after `since`, by schedule.
async fn overrides_of(st: &AppState, org: &str, schedule_ids: &[String], since: i64) -> Result<HashMap<String, Vec<OnCallOverride>>, ApiError> {
    let rows: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT o.schedule_id::text, o.id::text, o.user_id::text, u.name, (extract(epoch FROM o.start_at) * 1000)::bigint, (extract(epoch FROM o.end_at) * 1000)::bigint \
         FROM on_call_overrides o JOIN users u ON u.id = o.user_id WHERE o.org_id = $1 AND o.schedule_id::text = ANY($2) AND o.end_at > to_timestamp($3 / 1000.0) ORDER BY o.start_at LIMIT 5000",
    )
    .bind(org)
    .bind(schedule_ids)
    .bind(since as f64)
    .fetch_all(&st.postgres)
    .await?;
    let mut out: HashMap<String, Vec<OnCallOverride>> = HashMap::new();
    for (sid, id, user_id, user_name, start, end) in rows {
        out.entry(sid).or_default().push(OnCallOverride { id, user_id, user_name, start, end });
    }
    Ok(out)
}

fn spans(ovs: &[OnCallOverride]) -> Vec<OverrideSpan> {
    ovs.iter().map(|o| OverrideSpan { user_id: o.user_id.clone(), start: o.start, end: o.end }).collect()
}

async fn schedule_views(st: &AppState, org: &str, rows: Vec<ScheduleRow>) -> Result<Vec<OnCallSchedule>, ApiError> {
    let now = now_ms();
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut ovs = overrides_of(st, org, &ids, now - 31 * 86_400_000).await?;
    let mut everyone: Vec<String> = Vec::new();
    let mut computed = Vec::new();
    for r in rows {
        let layers = r.layers();
        let o = ovs.remove(&r.id).unwrap_or_default();
        let now_ids = on_call_at(&layers, &spans(&o), r.max(), r.tz(), now);
        let member_ids: Vec<String> = layers.iter().flat_map(|l| l.members.iter().cloned()).collect::<Vec<_>>();
        everyone.extend(now_ids.iter().cloned());
        everyone.extend(member_ids.iter().cloned());
        computed.push((r, layers, o, now_ids, member_ids));
    }
    everyone.sort();
    everyone.dedup();
    let by_id = users_by_id(st, org, &everyone).await?;
    Ok(computed
        .into_iter()
        .map(|(r, layers, overrides, now_ids, mut member_ids)| {
            member_ids.sort();
            member_ids.dedup();
            OnCallSchedule {
                on_call_now: pick_users(&by_id, &now_ids),
                members: pick_users(&by_id, &member_ids),
                id: r.id,
                name: r.name,
                time_zone: r.time_zone,
                team: r.team_handle,
                tags: r.tags,
                layers,
                max_concurrent: r.max_concurrent,
                overrides,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        })
        .collect())
}

async fn check_schedule(st: &AppState, org: &str, input: &mut OnCallScheduleInput) -> Result<(), ApiError> {
    input.name = text(&input.name, "The name", MAX_NAME)?;
    if parse_tz(&input.time_zone).is_none() {
        return Err(ApiError::BadRequest(format!("Unknown time zone {}", input.time_zone)));
    }
    input.team = optional_team(st, org, &input.team).await?;
    input.tags = clean_tags(&input.tags)?;
    if input.layers.len() > MAX_LAYERS {
        return Err(ApiError::BadRequest(format!("At most {MAX_LAYERS} layers")));
    }
    if input.max_concurrent.is_some_and(|m| !(1..=10).contains(&m)) {
        return Err(ApiError::BadRequest("Max concurrent responders is 1–10".into()));
    }
    let mut ids = HashSet::new();
    for (i, l) in input.layers.iter_mut().enumerate() {
        l.name = text(&l.name, "A layer name", MAX_NAME)?;
        if l.id.trim().is_empty() || l.id.len() > 64 || !ids.insert(l.id.clone()) {
            l.id = format!("layer-{}", i + 1);
            ids.insert(l.id.clone());
        }
        if l.members.len() > MAX_LAYER_MEMBERS || l.restrictions.len() > MAX_RESTRICTIONS {
            return Err(ApiError::BadRequest(format!("{}: at most {MAX_LAYER_MEMBERS} responders and {MAX_RESTRICTIONS} restrictions", l.name)));
        }
        if let Some(p) = layer_problem(l) {
            return Err(ApiError::BadRequest(p));
        }
    }
    let members: Vec<String> = input.layers.iter().flat_map(|l| l.members.iter().cloned()).collect();
    check_users(st, org, &members).await
}

#[utoipa::path(get, path = "/api/v1/on-call/schedules", operation_id = "list_on_call_schedules", responses((status = 200, body = Vec<OnCallSchedule>)), tag = "on-call")]
pub async fn list_schedules(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<OnCallSchedule>> {
    let rows = schedule_rows(&st, &ctx.org_id).await?;
    Ok(Json(schedule_views(&st, &ctx.org_id, rows).await?))
}

#[utoipa::path(get, path = "/api/v1/on-call/schedules/{id}", operation_id = "get_on_call_schedule", params(("id" = String, Path)), responses((status = 200, body = OnCallSchedule), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn get_schedule(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<OnCallSchedule> {
    let row = schedule_row(&st, &ctx.org_id, &id).await?;
    let mut views = schedule_views(&st, &ctx.org_id, vec![row]).await?;
    views.pop().map(Json).ok_or_else(|| ApiError::NotFound("Schedule not found".into()))
}

#[utoipa::path(post, path = "/api/v1/on-call/schedules", operation_id = "create_on_call_schedule", request_body = OnCallScheduleInput, responses((status = 200, body = OnCallSchedule)), tag = "on-call")]
pub async fn create_schedule(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<OnCallScheduleInput>) -> ApiResult<OnCallSchedule> {
    check_schedule(&st, &ctx.org_id, &mut input).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM on_call_schedules WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_SCHEDULES {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_SCHEDULES} schedules")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO on_call_schedules (org_id, name, time_zone, team_handle, tags, layers, max_concurrent, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid, $8::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(&input.time_zone)
    .bind(&input.team)
    .bind(&input.tags)
    .bind(serde_json::to_value(&input.layers).unwrap_or_default())
    .bind(input.max_concurrent)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "on_call.schedule.created", &input.name, &format!("{} layers", input.layers.len())).await;
    get_schedule(State(st), ctx, Path(id)).await
}

#[utoipa::path(put, path = "/api/v1/on-call/schedules/{id}", operation_id = "update_on_call_schedule", params(("id" = String, Path)), request_body = OnCallScheduleInput, responses((status = 200, body = OnCallSchedule), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn update_schedule(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<OnCallScheduleInput>) -> ApiResult<OnCallSchedule> {
    check_schedule(&st, &ctx.org_id, &mut input).await?;
    let n = sqlx::query(
        "UPDATE on_call_schedules SET name = $3, time_zone = $4, team_handle = $5, tags = $6, layers = $7, max_concurrent = $8, updated_by = $9::uuid, updated_at = now() WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.time_zone)
    .bind(&input.team)
    .bind(&input.tags)
    .bind(serde_json::to_value(&input.layers).unwrap_or_default())
    .bind(input.max_concurrent)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Schedule not found".into()));
    }
    ctx.audit(&st, "on_call.schedule.updated", &input.name, &format!("{} layers", input.layers.len())).await;
    get_schedule(State(st), ctx, Path(id)).await
}

#[utoipa::path(delete, path = "/api/v1/on-call/schedules/{id}", operation_id = "delete_on_call_schedule", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn delete_schedule(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM on_call_schedules WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Schedule not found".into())) };
    ctx.audit(&st, "on_call.schedule.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallLayerShifts {
    pub layer_id: String,
    pub shifts: Vec<OnCallShift>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallScheduleShifts {
    pub layers: Vec<OnCallLayerShifts>,
    pub final_shifts: Vec<OnCallFinalShift>,
    pub overrides: Vec<OnCallOverride>,
    /// Everyone named in the shifts.
    pub users: Vec<OnCallUser>,
}

fn shifts_of(
    layers: &[OnCallLayer],
    overrides: Vec<OnCallOverride>,
    max: Option<usize>,
    tz: chrono_tz::Tz,
    range: OnCallRange,
) -> (Vec<OnCallLayerShifts>, Vec<OnCallFinalShift>, Vec<OnCallOverride>) {
    let per_layer = layers.iter().map(|l| OnCallLayerShifts { layer_id: l.id.clone(), shifts: layer_shifts(l, tz, range.from, range.to) }).collect();
    let fin = final_schedule(layers, &spans(&overrides), max, tz, range.from, range.to);
    let overrides = overrides.into_iter().filter(|o| o.end > range.from && o.start < range.to).collect();
    (per_layer, fin, overrides)
}

async fn with_users(st: &AppState, org: &str, (layers, final_shifts, overrides): (Vec<OnCallLayerShifts>, Vec<OnCallFinalShift>, Vec<OnCallOverride>)) -> Result<OnCallScheduleShifts, ApiError> {
    let mut ids: Vec<String> = layers.iter().flat_map(|l| l.shifts.iter().map(|s| s.user_id.clone())).collect();
    ids.extend(final_shifts.iter().flat_map(|s| s.user_ids.iter().cloned()));
    ids.sort();
    ids.dedup();
    let by_id = users_by_id(st, org, &ids).await?;
    Ok(OnCallScheduleShifts { layers, final_shifts, overrides, users: pick_users(&by_id, &ids) })
}

/// The layers' shifts and the final schedule of a saved schedule over a range (≤ 93 days).
#[utoipa::path(post, path = "/api/v1/on-call/schedules/{id}/shifts", operation_id = "on_call_schedule_shifts", params(("id" = String, Path)), request_body = OnCallRange, responses((status = 200, body = OnCallScheduleShifts)), tag = "on-call")]
pub async fn schedule_shifts(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(range): Json<OnCallRange>) -> ApiResult<OnCallScheduleShifts> {
    range.check(MAX_RANGE_MS)?;
    let row = schedule_row(&st, &ctx.org_id, &id).await?;
    let ovs = overrides_of(&st, &ctx.org_id, std::slice::from_ref(&row.id), range.from).await?.remove(&row.id).unwrap_or_default();
    let computed = shifts_of(&row.layers(), ovs, row.max(), row.tz(), range);
    Ok(Json(with_users(&st, &ctx.org_id, computed).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPreviewRequest {
    pub schedule: OnCallScheduleInput,
    /// Include this saved schedule's overrides.
    #[serde(default)]
    pub schedule_id: Option<String>,
    pub range: OnCallRange,
}

/// Shifts of an unsaved schedule (the editor's timeline).
#[utoipa::path(post, path = "/api/v1/on-call/schedules/preview", operation_id = "preview_on_call_schedule", request_body = OnCallPreviewRequest, responses((status = 200, body = OnCallScheduleShifts)), tag = "on-call")]
pub async fn preview_schedule(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<OnCallPreviewRequest>) -> ApiResult<OnCallScheduleShifts> {
    req.range.check(MAX_RANGE_MS)?;
    let tz = parse_tz(&req.schedule.time_zone).ok_or_else(|| ApiError::BadRequest(format!("Unknown time zone {}", req.schedule.time_zone)))?;
    if req.schedule.layers.len() > MAX_LAYERS {
        return Err(ApiError::BadRequest(format!("At most {MAX_LAYERS} layers")));
    }
    let layers: Vec<OnCallLayer> = req.schedule.layers.into_iter().filter(|l| layer_problem(l).is_none() && l.members.len() <= MAX_LAYER_MEMBERS).collect();
    let ovs = match &req.schedule_id {
        Some(id) => overrides_of(&st, &ctx.org_id, std::slice::from_ref(id), req.range.from).await?.remove(id).unwrap_or_default(),
        None => Vec::new(),
    };
    let max = req.schedule.max_concurrent.map(|m| m.clamp(1, 10) as usize);
    Ok(Json(with_users(&st, &ctx.org_id, shifts_of(&layers, ovs, max, tz, req.range)).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateOnCallOverride {
    pub user_id: String,
    pub start: i64,
    pub end: i64,
}

#[utoipa::path(post, path = "/api/v1/on-call/schedules/{id}/overrides", operation_id = "create_on_call_override", params(("id" = String, Path)), request_body = CreateOnCallOverride, responses((status = 200, body = OnCallOverride)), tag = "on-call")]
pub async fn create_override(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(req): Json<CreateOnCallOverride>) -> ApiResult<OnCallOverride> {
    if req.end <= req.start || req.end - req.start > 366 * 86_400_000 {
        return Err(ApiError::BadRequest("An override ends after it starts and lasts at most a year".into()));
    }
    let row = schedule_row(&st, &ctx.org_id, &id).await?;
    check_users(&st, &ctx.org_id, std::slice::from_ref(&req.user_id)).await?;
    let o: OnCallOverride = sqlx::query_as(
        "WITH o AS (INSERT INTO on_call_overrides (org_id, schedule_id, user_id, start_at, end_at, created_by) \
         VALUES ($1, $2::uuid, $3::uuid, to_timestamp($4 / 1000.0), to_timestamp($5 / 1000.0), $6::uuid) RETURNING *) \
         SELECT o.id::text AS id, o.user_id::text AS user_id, u.name AS user_name, (extract(epoch FROM o.start_at) * 1000)::bigint AS start, (extract(epoch FROM o.end_at) * 1000)::bigint AS \"end\" \
         FROM o JOIN users u ON u.id = o.user_id",
    )
    .bind(&ctx.org_id)
    .bind(&row.id)
    .bind(&req.user_id)
    .bind(req.start as f64)
    .bind(req.end as f64)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "on_call.override.created", &row.name, &o.user_name).await;
    Ok(Json(o))
}

#[utoipa::path(delete, path = "/api/v1/on-call/schedules/{id}/overrides/{override_id}", operation_id = "delete_on_call_override", params(("id" = String, Path), ("override_id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn delete_override(State(st): State<AppState>, Writer(ctx): Writer, Path((id, override_id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    let row = schedule_row(&st, &ctx.org_id, &id).await?;
    let n = sqlx::query("DELETE FROM on_call_overrides WHERE org_id = $1 AND schedule_id::text = $2 AND id::text = $3")
        .bind(&ctx.org_id)
        .bind(&row.id)
        .bind(&override_id)
        .execute(&st.postgres)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Override not found".into()));
    }
    ctx.audit(&st, "on_call.override.deleted", &row.name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallMyShift {
    pub schedule_id: String,
    pub schedule_name: String,
    pub start: i64,
    pub end: i64,
}

/// The stretches of a final schedule that include `user`, joined when contiguous.
fn shifts_for(user: &str, fin: &[OnCallFinalShift]) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::new();
    for s in fin.iter().filter(|s| s.user_ids.iter().any(|u| u == user)) {
        match out.last_mut() {
            Some(last) if last.1 == s.start => last.1 = s.end,
            _ => out.push((s.start, s.end)),
        }
    }
    out
}

/// The signed-in member's shifts across every schedule (Summary → My Shifts, calendar export).
#[utoipa::path(post, path = "/api/v1/on-call/my-shifts", operation_id = "my_on_call_shifts", request_body = OnCallRange, responses((status = 200, body = Vec<OnCallMyShift>)), tag = "on-call")]
pub async fn my_shifts(State(st): State<AppState>, ctx: AuthContext, Json(range): Json<OnCallRange>) -> ApiResult<Vec<OnCallMyShift>> {
    range.check(MAX_RANGE_MS)?;
    let rows = schedule_rows(&st, &ctx.org_id).await?;
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut ovs = overrides_of(&st, &ctx.org_id, &ids, range.from).await?;
    let mut out = Vec::new();
    for r in rows {
        let o = ovs.remove(&r.id).unwrap_or_default();
        let fin = final_schedule(&r.layers(), &spans(&o), r.max(), r.tz(), range.from, range.to);
        for (start, end) in shifts_for(&ctx.user_id, &fin) {
            out.push(OnCallMyShift { schedule_id: r.id.clone(), schedule_name: r.name.clone(), start, end });
        }
    }
    out.sort_by_key(|s| s.start);
    Ok(Json(out))
}

// ================================================================ escalation policies

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPolicy {
    pub id: String,
    pub name: String,
    pub team: Option<String>,
    pub tags: Vec<String>,
    pub steps: Vec<OnCallStep>,
    /// Times the policy starts over after its last step.
    pub repeat: i32,
    pub resolve_after_minutes: Option<i32>,
    /// Teams whose pages follow this policy.
    pub used_by: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPolicyInput {
    pub name: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub steps: Vec<OnCallStep>,
    #[serde(default)]
    pub repeat: i32,
    #[serde(default)]
    pub resolve_after_minutes: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct PolicyRow {
    id: String,
    name: String,
    team_handle: Option<String>,
    tags: Vec<String>,
    steps: Value,
    repeat_count: i32,
    resolve_after_minutes: Option<i32>,
    used_by: Vec<String>,
    created_at: i64,
    updated_at: i64,
}

impl PolicyRow {
    fn steps(&self) -> Vec<OnCallStep> {
        serde_json::from_value(self.steps.clone()).unwrap_or_default()
    }
    fn view(self) -> OnCallPolicy {
        OnCallPolicy {
            steps: self.steps(),
            id: self.id,
            name: self.name,
            team: self.team_handle,
            tags: self.tags,
            repeat: self.repeat_count,
            resolve_after_minutes: self.resolve_after_minutes,
            used_by: self.used_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const POLICY_SELECT: &str = "SELECT p.id::text AS id, p.name, p.team_handle, p.tags, p.steps, p.repeat_count, p.resolve_after_minutes, \
    COALESCE((SELECT array_agg(r.team_handle ORDER BY r.team_handle) FROM on_call_team_routing r WHERE r.org_id = p.org_id AND r.policy_id = p.id), '{}') AS used_by, \
    (extract(epoch FROM p.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM p.updated_at) * 1000)::bigint AS updated_at FROM on_call_escalation_policies p";

async fn policy_row(st: &AppState, org: &str, id: &str) -> Result<Option<PolicyRow>, ApiError> {
    Ok(sqlx::query_as(&format!("{POLICY_SELECT} WHERE p.org_id = $1 AND p.id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?)
}

async fn check_policy(st: &AppState, org: &str, input: &mut OnCallPolicyInput) -> Result<(), ApiError> {
    input.name = text(&input.name, "The name", MAX_NAME)?;
    input.team = optional_team(st, org, &input.team).await?;
    input.tags = clean_tags(&input.tags)?;
    if let Some(p) = steps_problem(&input.steps, input.repeat, input.resolve_after_minutes) {
        return Err(ApiError::BadRequest(p));
    }
    let targets: Vec<&OnCallTarget> = input.steps.iter().flat_map(|s| s.targets.iter()).collect();
    let users: Vec<String> = targets.iter().filter(|t| t.kind == OnCallTargetKind::User).map(|t| t.id.clone()).collect();
    check_users(st, org, &users).await?;
    for t in targets.iter().filter(|t| t.kind == OnCallTargetKind::Team) {
        check_team(st, org, &t.id).await?;
    }
    let schedules: Vec<String> = targets.iter().filter(|t| t.kind == OnCallTargetKind::Schedule).map(|t| t.id.clone()).collect();
    if !schedules.is_empty() {
        let found: Vec<(String,)> = sqlx::query_as("SELECT id::text FROM on_call_schedules WHERE org_id = $1 AND id::text = ANY($2)").bind(org).bind(&schedules).fetch_all(&st.postgres).await?;
        if let Some(missing) = schedules.iter().find(|s| !found.iter().any(|f| &f.0 == *s)) {
            return Err(ApiError::BadRequest(format!("Unknown schedule {missing}")));
        }
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/on-call/escalation-policies", operation_id = "list_on_call_policies", responses((status = 200, body = Vec<OnCallPolicy>)), tag = "on-call")]
pub async fn list_policies(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<OnCallPolicy>> {
    let rows: Vec<PolicyRow> = sqlx::query_as(&format!("{POLICY_SELECT} WHERE p.org_id = $1 ORDER BY lower(p.name) LIMIT {MAX_POLICIES}")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(PolicyRow::view).collect()))
}

#[utoipa::path(get, path = "/api/v1/on-call/escalation-policies/{id}", operation_id = "get_on_call_policy", params(("id" = String, Path)), responses((status = 200, body = OnCallPolicy), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn get_policy(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<OnCallPolicy> {
    policy_row(&st, &ctx.org_id, &id).await?.map(|r| Json(r.view())).ok_or_else(|| ApiError::NotFound("Escalation policy not found".into()))
}

#[utoipa::path(post, path = "/api/v1/on-call/escalation-policies", operation_id = "create_on_call_policy", request_body = OnCallPolicyInput, responses((status = 200, body = OnCallPolicy)), tag = "on-call")]
pub async fn create_policy(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<OnCallPolicyInput>) -> ApiResult<OnCallPolicy> {
    check_policy(&st, &ctx.org_id, &mut input).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM on_call_escalation_policies WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_POLICIES {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_POLICIES} escalation policies")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO on_call_escalation_policies (org_id, name, team_handle, tags, steps, repeat_count, resolve_after_minutes, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid, $8::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(&input.team)
    .bind(&input.tags)
    .bind(serde_json::to_value(&input.steps).unwrap_or_default())
    .bind(input.repeat)
    .bind(input.resolve_after_minutes)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "on_call.escalation_policy.created", &input.name, &format!("{} steps", input.steps.len())).await;
    get_policy(State(st), ctx, Path(id)).await
}

#[utoipa::path(put, path = "/api/v1/on-call/escalation-policies/{id}", operation_id = "update_on_call_policy", params(("id" = String, Path)), request_body = OnCallPolicyInput, responses((status = 200, body = OnCallPolicy), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn update_policy(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<OnCallPolicyInput>) -> ApiResult<OnCallPolicy> {
    check_policy(&st, &ctx.org_id, &mut input).await?;
    let n = sqlx::query(
        "UPDATE on_call_escalation_policies SET name = $3, team_handle = $4, tags = $5, steps = $6, repeat_count = $7, resolve_after_minutes = $8, updated_by = $9::uuid, updated_at = now() \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.team)
    .bind(&input.tags)
    .bind(serde_json::to_value(&input.steps).unwrap_or_default())
    .bind(input.repeat)
    .bind(input.resolve_after_minutes)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Escalation policy not found".into()));
    }
    ctx.audit(&st, "on_call.escalation_policy.updated", &input.name, &format!("{} steps", input.steps.len())).await;
    get_policy(State(st), ctx, Path(id)).await
}

#[utoipa::path(delete, path = "/api/v1/on-call/escalation-policies/{id}", operation_id = "delete_on_call_policy", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn delete_policy(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("DELETE FROM on_call_escalation_policies WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Escalation policy not found".into())) };
    ctx.audit(&st, "on_call.escalation_policy.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ teams

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallTeam {
    pub handle: String,
    pub name: String,
    pub member_count: i64,
    /// The escalation policy the team's pages follow (none: every member is paged).
    pub policy: Option<OnCallRef>,
    /// Schedules owned by the team.
    pub schedules: Vec<OnCallRef>,
    /// On call now on the team's schedules.
    pub on_call_now: Vec<OnCallUser>,
    pub open_pages: i64,
    /// The signed-in member belongs to the team.
    pub mine: bool,
}

#[derive(sqlx::FromRow)]
struct TeamRow {
    handle: String,
    name: String,
    member_count: i64,
    policy_id: Option<String>,
    policy_name: Option<String>,
    open_pages: i64,
    mine: bool,
}

#[utoipa::path(get, path = "/api/v1/on-call/teams", operation_id = "list_on_call_teams", responses((status = 200, body = Vec<OnCallTeam>)), tag = "on-call")]
pub async fn list_teams(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<OnCallTeam>> {
    let rows: Vec<TeamRow> = sqlx::query_as(
        "SELECT t.handle, t.name, (SELECT count(*) FROM team_members m WHERE m.org_id = t.org_id AND m.handle = t.handle) AS member_count, \
         p.id::text AS policy_id, p.name AS policy_name, \
         (SELECT count(*) FROM on_call_pages g WHERE g.org_id = t.org_id AND g.team_handle = t.handle AND g.status <> 'resolved') AS open_pages, \
         EXISTS (SELECT 1 FROM team_members m WHERE m.org_id = t.org_id AND m.handle = t.handle AND m.user_id::text = $2) AS mine \
         FROM teams t LEFT JOIN on_call_team_routing r ON r.org_id = t.org_id AND r.team_handle = t.handle LEFT JOIN on_call_escalation_policies p ON p.id = r.policy_id \
         WHERE t.org_id = $1 ORDER BY lower(t.name) LIMIT 1000",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .fetch_all(&st.postgres)
    .await?;
    let schedules = schedule_views(&st, &ctx.org_id, schedule_rows(&st, &ctx.org_id).await?).await?;
    let out = rows
        .into_iter()
        .map(|t| {
            let owned: Vec<&OnCallSchedule> = schedules.iter().filter(|s| s.team.as_deref() == Some(t.handle.as_str())).collect();
            let mut on_call_now: Vec<OnCallUser> = Vec::new();
            for u in owned.iter().flat_map(|s| s.on_call_now.iter()) {
                if !on_call_now.contains(u) {
                    on_call_now.push(u.clone());
                }
            }
            OnCallTeam {
                schedules: owned.iter().map(|s| OnCallRef { id: s.id.clone(), name: s.name.clone() }).collect(),
                on_call_now,
                policy: t.policy_id.zip(t.policy_name).map(|(id, name)| OnCallRef { id, name }),
                handle: t.handle,
                name: t.name,
                member_count: t.member_count,
                open_pages: t.open_pages,
                mine: t.mine,
            }
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallRouting {
    /// None: page every member of the team.
    pub policy_id: Option<String>,
}

/// Which escalation policy the team's pages follow.
#[utoipa::path(put, path = "/api/v1/on-call/teams/{handle}/routing", operation_id = "set_on_call_routing", params(("handle" = String, Path)), request_body = OnCallRouting, responses((status = 204)), tag = "on-call")]
pub async fn set_routing(State(st): State<AppState>, Writer(ctx): Writer, Path(handle): Path<String>, Json(req): Json<OnCallRouting>) -> Result<StatusCode, ApiError> {
    check_team(&st, &ctx.org_id, &handle).await.map_err(|_| ApiError::NotFound(format!("Team @{handle} not found")))?;
    match req.policy_id.as_deref().filter(|p| !p.is_empty()) {
        None => {
            sqlx::query("DELETE FROM on_call_team_routing WHERE org_id = $1 AND team_handle = $2").bind(&ctx.org_id).bind(&handle).execute(&st.postgres).await?;
            ctx.audit(&st, "on_call.routing.updated", &handle, "page every member").await;
        }
        Some(pid) => {
            let Some(p) = policy_row(&st, &ctx.org_id, pid).await? else { return Err(ApiError::BadRequest("Unknown escalation policy".into())) };
            sqlx::query(
                "INSERT INTO on_call_team_routing (org_id, team_handle, policy_id, updated_by) VALUES ($1, $2, $3::uuid, $4::uuid) \
                 ON CONFLICT (org_id, team_handle) DO UPDATE SET policy_id = EXCLUDED.policy_id, updated_by = EXCLUDED.updated_by, updated_at = now()",
            )
            .bind(&ctx.org_id)
            .bind(&handle)
            .bind(&p.id)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await?;
            ctx.audit(&st, "on_call.routing.updated", &handle, &p.name).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ pages

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPage {
    pub id: i64,
    pub title: String,
    pub description: String,
    /// high | low
    pub urgency: String,
    pub status: OnCallPageStatus,
    pub team: Option<String>,
    pub policy: Option<OnCallRef>,
    /// manual | incident | monitor
    pub source: String,
    /// IR-<n> of the incident that paged.
    pub incident_id: Option<i32>,
    pub monitor_id: Option<i64>,
    /// Everyone notified so far, in order.
    pub responders: Vec<OnCallUser>,
    pub created_by: String,
    pub created_at: i64,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<i64>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<i64>,
    /// When the next escalation step runs (triggered pages).
    pub next_escalation_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct PageRow {
    id: i64,
    title: String,
    description: String,
    urgency: String,
    status: String,
    team_handle: Option<String>,
    policy_id: Option<String>,
    policy_name: Option<String>,
    source: String,
    incident_public_id: Option<i32>,
    monitor_id: Option<i64>,
    created_by: String,
    created_at: i64,
    acknowledged_by: Option<String>,
    acknowledged_at: Option<i64>,
    resolved_by: Option<String>,
    resolved_at: Option<i64>,
    next_escalation_at: Option<i64>,
}

const PAGE_SELECT: &str = "SELECT g.id, g.title, g.description, g.urgency, g.status, g.team_handle, g.policy_id::text AS policy_id, p.name AS policy_name, g.source, \
    i.public_id AS incident_public_id, g.monitor_id, g.created_by, (extract(epoch FROM g.created_at) * 1000)::bigint AS created_at, g.acknowledged_by, \
    (extract(epoch FROM g.acknowledged_at) * 1000)::bigint AS acknowledged_at, g.resolved_by, (extract(epoch FROM g.resolved_at) * 1000)::bigint AS resolved_at, \
    CASE WHEN g.status = 'triggered' THEN (extract(epoch FROM g.next_escalation_at) * 1000)::bigint END AS next_escalation_at \
    FROM on_call_pages g LEFT JOIN on_call_escalation_policies p ON p.id = g.policy_id LEFT JOIN incidents i ON i.id = g.incident_id";

async fn page_views(st: &AppState, org: &str, rows: Vec<PageRow>) -> Result<Vec<OnCallPage>, ApiError> {
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let responders: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT r.page_id, u.id::text, u.name, u.email FROM on_call_page_responders r JOIN users u ON u.id = r.user_id JOIN on_call_pages g ON g.id = r.page_id \
         WHERE g.org_id = $1 AND r.page_id = ANY($2) ORDER BY r.notified_at",
    )
    .bind(org)
    .bind(&ids)
    .fetch_all(&st.postgres)
    .await?;
    let mut by_page: HashMap<i64, Vec<OnCallUser>> = HashMap::new();
    for (pid, id, name, email) in responders {
        by_page.entry(pid).or_default().push(OnCallUser { id, name, email });
    }
    Ok(rows
        .into_iter()
        .map(|r| OnCallPage {
            responders: by_page.remove(&r.id).unwrap_or_default(),
            id: r.id,
            title: r.title,
            description: r.description,
            urgency: r.urgency,
            status: OnCallPageStatus::parse(&r.status).unwrap_or(OnCallPageStatus::Triggered),
            team: r.team_handle,
            policy: r.policy_id.zip(r.policy_name).map(|(id, name)| OnCallRef { id, name }),
            source: r.source,
            incident_id: r.incident_public_id,
            monitor_id: r.monitor_id,
            created_by: r.created_by,
            created_at: r.created_at,
            acknowledged_by: r.acknowledged_by,
            acknowledged_at: r.acknowledged_at,
            resolved_by: r.resolved_by,
            resolved_at: r.resolved_at,
            next_escalation_at: r.next_escalation_at,
        })
        .collect())
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPageSearch {
    /// active (triggered or acknowledged) | triggered | acknowledged | resolved | all
    pub status: String,
    #[serde(default)]
    pub teams: Vec<String>,
    /// Only pages that notified the signed-in member.
    #[serde(default)]
    pub mine: bool,
    /// Words of the title.
    #[serde(default)]
    pub query: String,
    /// Created within (Unix ms); none: any time.
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    /// At most this many pages (1–500, default 200).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema, Default, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPageCounts {
    pub active: i64,
    pub triggered: i64,
    pub acknowledged: i64,
    pub resolved: i64,
    pub all: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPagesResult {
    pub pages: Vec<OnCallPage>,
    /// Per status, with the other filters applied.
    pub counts: OnCallPageCounts,
}

fn push_page_filters(qb: &mut QueryBuilder<Postgres>, org: &str, me: &str, req: &OnCallPageSearch) {
    qb.push(" WHERE g.org_id = ").push_bind(org.to_string());
    if !req.teams.is_empty() {
        qb.push(" AND g.team_handle = ANY(").push_bind(req.teams.clone()).push(")");
    }
    if req.mine {
        qb.push(" AND EXISTS (SELECT 1 FROM on_call_page_responders r WHERE r.page_id = g.id AND r.user_id::text = ").push_bind(me.to_string()).push(")");
    }
    for word in req.query.split_whitespace().take(10) {
        let escaped = word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        qb.push(" AND g.title ILIKE ").push_bind(format!("%{escaped}%"));
    }
    if let Some(from) = req.from {
        qb.push(" AND g.created_at >= to_timestamp(").push_bind(from as f64).push(" / 1000.0)");
    }
    if let Some(to) = req.to {
        qb.push(" AND g.created_at <= to_timestamp(").push_bind(to as f64).push(" / 1000.0)");
    }
}

#[utoipa::path(post, path = "/api/v1/on-call/pages/search", operation_id = "search_on_call_pages", request_body = OnCallPageSearch, responses((status = 200, body = OnCallPagesResult)), tag = "on-call")]
pub async fn search_pages(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<OnCallPageSearch>) -> ApiResult<OnCallPagesResult> {
    let status_sql = match req.status.as_str() {
        "active" => " AND g.status <> 'resolved'",
        "triggered" => " AND g.status = 'triggered'",
        "acknowledged" => " AND g.status = 'acknowledged'",
        "resolved" => " AND g.status = 'resolved'",
        "all" => "",
        _ => return Err(ApiError::BadRequest("status is active, triggered, acknowledged, resolved or all".into())),
    };
    let mut counts_q: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT count(*) FILTER (WHERE g.status <> 'resolved') AS active, count(*) FILTER (WHERE g.status = 'triggered') AS triggered, \
         count(*) FILTER (WHERE g.status = 'acknowledged') AS acknowledged, count(*) FILTER (WHERE g.status = 'resolved') AS resolved, count(*) AS \"all\" FROM on_call_pages g",
    );
    push_page_filters(&mut counts_q, &ctx.org_id, &ctx.user_id, &req);
    let counts: OnCallPageCounts = counts_q.build_query_as().fetch_one(&st.postgres).await?;
    let mut q: QueryBuilder<Postgres> = QueryBuilder::new(PAGE_SELECT);
    push_page_filters(&mut q, &ctx.org_id, &ctx.user_id, &req);
    q.push(status_sql).push(" ORDER BY g.created_at DESC LIMIT ").push_bind(req.limit.unwrap_or(200).clamp(1, 500));
    let rows: Vec<PageRow> = q.build_query_as().fetch_all(&st.postgres).await?;
    Ok(Json(OnCallPagesResult { pages: page_views(&st, &ctx.org_id, rows).await?, counts }))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPageEvent {
    pub id: i64,
    /// triggered | notified | escalated | acknowledged | resolved | reassigned | failed
    pub kind: String,
    pub actor: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallPageDetail {
    pub page: OnCallPage,
    pub timeline: Vec<OnCallPageEvent>,
    /// The signed-in member may acknowledge and resolve it.
    pub can_respond: bool,
}

async fn page_view(st: &AppState, org: &str, id: i64) -> Result<OnCallPage, ApiError> {
    let row: Option<PageRow> = sqlx::query_as(&format!("{PAGE_SELECT} WHERE g.org_id = $1 AND g.id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    let row = row.ok_or_else(|| ApiError::NotFound("Page not found".into()))?;
    page_views(st, org, vec![row]).await?.pop().ok_or_else(|| ApiError::NotFound("Page not found".into()))
}

async fn may_respond(st: &AppState, ctx: &AuthContext, page_id: i64) -> Result<bool, ApiError> {
    if ctx.access >= Access::Standard {
        return Ok(true);
    }
    let paged: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM on_call_page_responders r JOIN on_call_pages g ON g.id = r.page_id WHERE g.org_id = $1 AND r.page_id = $2 AND r.user_id::text = $3")
        .bind(&ctx.org_id)
        .bind(page_id)
        .bind(&ctx.user_id)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(paged.is_some())
}

#[utoipa::path(get, path = "/api/v1/on-call/pages/{id}", operation_id = "get_on_call_page", params(("id" = i64, Path)), responses((status = 200, body = OnCallPageDetail), (status = 404, body = crate::error::ErrorBody)), tag = "on-call")]
pub async fn get_page(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<i64>) -> ApiResult<OnCallPageDetail> {
    let page = page_view(&st, &ctx.org_id, id).await?;
    let timeline: Vec<OnCallPageEvent> = sqlx::query_as(
        "SELECT e.id, e.kind, e.actor, e.body, (extract(epoch FROM e.created_at) * 1000)::bigint AS created_at FROM on_call_page_events e JOIN on_call_pages g ON g.id = e.page_id \
         WHERE g.org_id = $1 AND e.page_id = $2 ORDER BY e.id",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .fetch_all(&st.postgres)
    .await?;
    let can_respond = may_respond(&st, &ctx, id).await?;
    Ok(Json(OnCallPageDetail { page, timeline, can_respond }))
}

/// What a new page is about and where it goes.
pub(crate) struct NewPage {
    pub org: String,
    pub title: String,
    pub description: String,
    pub urgency: String,
    pub team: Option<String>,
    pub policy_id: Option<String>,
    pub source: &'static str,
    pub incident_pk: Option<i64>,
    pub monitor_id: Option<i64>,
    pub monitor_group: Option<String>,
    pub created_by: String,
    pub created_by_id: Option<String>,
}

/// The policy a page follows: the one asked for, else the team's routing (none: the team's members are paged).
async fn route(st: &AppState, org: &str, team: Option<&str>, policy_id: Option<&str>) -> Result<Option<PolicyRow>, ApiError> {
    if let Some(pid) = policy_id {
        return policy_row(st, org, pid).await?.map(Some).ok_or_else(|| ApiError::BadRequest("Unknown escalation policy".into()));
    }
    let Some(team) = team else { return Ok(None) };
    let routed: Option<(String,)> = sqlx::query_as("SELECT policy_id::text FROM on_call_team_routing WHERE org_id = $1 AND team_handle = $2").bind(org).bind(team).fetch_optional(&st.postgres).await?;
    match routed {
        Some((pid,)) => policy_row(st, org, &pid).await,
        None => Ok(None),
    }
}

async fn page_event<'e, E: sqlx::PgExecutor<'e>>(db: E, page_id: i64, kind: &str, actor: &str, body: &str) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO on_call_page_events (page_id, kind, actor, body) VALUES ($1, $2, $3, $4)").bind(page_id).bind(kind).bind(actor).bind(body).execute(db).await?;
    Ok(())
}

/// Creates a page; the escalation worker notifies its first step within seconds.
pub(crate) async fn create_page_row(st: &AppState, p: NewPage) -> Result<i64, ApiError> {
    if p.team.is_none() && p.policy_id.is_none() {
        return Err(ApiError::BadRequest("Page a team or an escalation policy".into()));
    }
    if let Some(t) = &p.team {
        check_team(st, &p.org, t).await?;
    }
    let policy = route(st, &p.org, p.team.as_deref(), p.policy_id.as_deref()).await?;
    let resolve_after = policy.as_ref().and_then(|x| x.resolve_after_minutes);
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO on_call_pages (org_id, title, description, urgency, team_handle, policy_id, source, incident_id, monitor_id, monitor_group, created_by, created_by_id, resolve_at) \
         VALUES ($1, $2, $3, $4, $5, $6::uuid, $7, $8, $9, $10, $11, $12::uuid, CASE WHEN $13::int IS NULL THEN NULL ELSE now() + make_interval(mins => $13::int) END) RETURNING id",
    )
    .bind(&p.org)
    .bind(&p.title)
    .bind(&p.description)
    .bind(&p.urgency)
    .bind(&p.team)
    .bind(policy.as_ref().map(|x| x.id.clone()))
    .bind(p.source)
    .bind(p.incident_pk)
    .bind(p.monitor_id)
    .bind(&p.monitor_group)
    .bind(&p.created_by)
    .bind(&p.created_by_id)
    .bind(resolve_after)
    .fetch_one(&st.postgres)
    .await?;
    let to = match (&p.team, &policy) {
        (Some(t), Some(x)) => format!("Paged @{t} ({})", x.name),
        (Some(t), None) => format!("Paged every member of @{t} (the team has no escalation policy)"),
        (None, Some(x)) => format!("Paged {}", x.name),
        (None, None) => String::new(),
    };
    page_event(&st.postgres, id, "triggered", &p.created_by, &to).await?;
    Ok(id)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateOnCallPage {
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// high | low (default high)
    #[serde(default)]
    pub urgency: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub policy_id: Option<String>,
}

fn urgency(u: Option<&str>) -> Result<String, ApiError> {
    match u.unwrap_or("high") {
        "high" => Ok("high".into()),
        "low" => Ok("low".into()),
        _ => Err(ApiError::BadRequest("urgency is high or low".into())),
    }
}

#[utoipa::path(post, path = "/api/v1/on-call/pages", operation_id = "create_on_call_page", request_body = CreateOnCallPage, responses((status = 200, body = OnCallPage)), tag = "on-call")]
pub async fn create_page(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<CreateOnCallPage>) -> ApiResult<OnCallPage> {
    let description = req.description.trim().chars().take(MAX_DESCRIPTION).collect();
    let id = create_page_row(
        &st,
        NewPage {
            org: ctx.org_id.clone(),
            title: text(&req.title, "The title", MAX_TITLE)?,
            description,
            urgency: urgency(req.urgency.as_deref())?,
            team: req.team.filter(|t| !t.is_empty()),
            policy_id: req.policy_id.filter(|p| !p.is_empty()),
            source: "manual",
            incident_pk: None,
            monitor_id: None,
            monitor_group: None,
            created_by: ctx.name.clone(),
            created_by_id: Some(ctx.user_id.clone()),
        },
    )
    .await?;
    Ok(Json(page_view(&st, &ctx.org_id, id).await?))
}

/// Acknowledges or resolves a page (and the incident page it came from).
pub(crate) async fn set_page_status(st: &AppState, org: &str, page_id: i64, action: PageAction, actor: &str) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM on_call_pages WHERE org_id = $1 AND id = $2 FOR UPDATE").bind(org).bind(page_id).fetch_optional(&mut *tx).await?;
    let Some((status,)) = row else { return Err(ApiError::NotFound("Page not found".into())) };
    let current = OnCallPageStatus::parse(&status).unwrap_or(OnCallPageStatus::Triggered);
    let next = transition(current, action).map_err(|e| ApiError::BadRequest(e.into()))?;
    match next {
        OnCallPageStatus::Acknowledged => {
            sqlx::query("UPDATE on_call_pages SET status = 'acknowledged', acknowledged_by = $2, acknowledged_at = now(), next_escalation_at = NULL WHERE id = $1")
                .bind(page_id)
                .bind(actor)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE incident_pages SET status = 'acknowledged', acknowledged_by = $2, acknowledged_at = now() WHERE oncall_page_id = $1 AND status = 'triggered'")
                .bind(page_id)
                .bind(actor)
                .execute(&mut *tx)
                .await?;
            page_event(&mut *tx, page_id, "acknowledged", actor, "Acknowledged; escalation stopped").await?;
        }
        OnCallPageStatus::Resolved => {
            sqlx::query(
                "UPDATE on_call_pages SET status = 'resolved', resolved_by = $2, resolved_at = now(), next_escalation_at = NULL, \
                 acknowledged_by = COALESCE(acknowledged_by, $2), acknowledged_at = COALESCE(acknowledged_at, now()) WHERE id = $1",
            )
            .bind(page_id)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE incident_pages SET status = 'resolved', resolved_at = now(), acknowledged_by = COALESCE(acknowledged_by, $2), acknowledged_at = COALESCE(acknowledged_at, now()) \
                 WHERE oncall_page_id = $1 AND status <> 'resolved'",
            )
            .bind(page_id)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
            page_event(&mut *tx, page_id, "resolved", actor, "Resolved").await?;
        }
        OnCallPageStatus::Triggered => return Err(ApiError::BadRequest("Use reassign to page again".into())),
    }
    tx.commit().await?;
    Ok(())
}

async fn respond(st: &AppState, ctx: &AuthContext, id: i64, action: PageAction) -> ApiResult<OnCallPage> {
    page_view(st, &ctx.org_id, id).await?;
    if !may_respond(st, ctx, id).await? {
        return Err(ApiError::Forbidden("Only responders of this page, standard users and admins can do that".into()));
    }
    set_page_status(st, &ctx.org_id, id, action, &ctx.name).await?;
    Ok(Json(page_view(st, &ctx.org_id, id).await?))
}

#[utoipa::path(post, path = "/api/v1/on-call/pages/{id}/acknowledge", operation_id = "acknowledge_on_call_page", params(("id" = i64, Path)), responses((status = 200, body = OnCallPage)), tag = "on-call")]
pub async fn acknowledge_page(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<i64>) -> ApiResult<OnCallPage> {
    respond(&st, &ctx, id, PageAction::Acknowledge).await
}

#[utoipa::path(post, path = "/api/v1/on-call/pages/{id}/resolve", operation_id = "resolve_on_call_page", params(("id" = i64, Path)), responses((status = 200, body = OnCallPage)), tag = "on-call")]
pub async fn resolve_page(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<i64>) -> ApiResult<OnCallPage> {
    respond(&st, &ctx, id, PageAction::Resolve).await
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReassignOnCallPage {
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub policy_id: Option<String>,
}

/// Pages another team or policy: back to triggered, from the first step.
#[utoipa::path(post, path = "/api/v1/on-call/pages/{id}/reassign", operation_id = "reassign_on_call_page", params(("id" = i64, Path)), request_body = ReassignOnCallPage, responses((status = 200, body = OnCallPage)), tag = "on-call")]
pub async fn reassign_page(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i64>, Json(req): Json<ReassignOnCallPage>) -> ApiResult<OnCallPage> {
    let team = optional_team(&st, &ctx.org_id, &req.team).await?;
    let policy_id = req.policy_id.filter(|p| !p.is_empty());
    if team.is_none() && policy_id.is_none() {
        return Err(ApiError::BadRequest("Choose a team or an escalation policy".into()));
    }
    let policy = route(&st, &ctx.org_id, team.as_deref(), policy_id.as_deref()).await?;
    let mut tx = st.postgres.begin().await?;
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM on_call_pages WHERE org_id = $1 AND id = $2 FOR UPDATE").bind(&ctx.org_id).bind(id).fetch_optional(&mut *tx).await?;
    let Some((status,)) = row else { return Err(ApiError::NotFound("Page not found".into())) };
    transition(OnCallPageStatus::parse(&status).unwrap_or(OnCallPageStatus::Triggered), PageAction::Reassign).map_err(|e| ApiError::BadRequest(e.into()))?;
    let resolve_after = policy.as_ref().and_then(|p| p.resolve_after_minutes);
    sqlx::query(
        "UPDATE on_call_pages SET status = 'triggered', team_handle = $2, policy_id = $3::uuid, step = 0, round = 0, next_escalation_at = now(), acknowledged_by = NULL, acknowledged_at = NULL, \
         resolve_at = CASE WHEN $4::int IS NULL THEN NULL ELSE now() + make_interval(mins => $4::int) END WHERE id = $1",
    )
    .bind(id)
    .bind(&team)
    .bind(policy.as_ref().map(|p| p.id.clone()))
    .bind(resolve_after)
    .execute(&mut *tx)
    .await?;
    let target = [team.as_ref().map(|t| format!("@{t}")), policy.as_ref().map(|p| p.name.clone())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    page_event(&mut *tx, id, "reassigned", &ctx.name, &format!("Reassigned to {target}")).await?;
    tx.commit().await?;
    Ok(Json(page_view(&st, &ctx.org_id, id).await?))
}

// ================================================================ monitors and incidents

/// `@oncall-<team>` in a monitor message: an alert pages the team (once per monitor group while
/// the page is open), a recovery resolves the open pages of that group.
pub(crate) async fn page_from_monitor(st: &AppState, team: &str, n: &Notice) -> Outcome {
    match check_team(st, &n.org, team).await {
        Ok(()) => {}
        Err(ApiError::BadRequest(m)) => return Outcome::Unknown(format!("{m}: @oncall-<team> names a team of the organization")),
        Err(e) => return Outcome::Failed(e.to_string()),
    }
    if n.test {
        return Outcome::Sent;
    }
    let open: Result<Vec<(i64,)>, _> =
        sqlx::query_as("SELECT id FROM on_call_pages WHERE org_id = $1 AND monitor_id = $2 AND monitor_group IS NOT DISTINCT FROM $3 AND team_handle = $4 AND status <> 'resolved'")
            .bind(&n.org)
            .bind(n.monitor_id)
            .bind(&n.group)
            .bind(team)
            .fetch_all(&st.postgres)
            .await;
    let open = match open {
        Ok(o) => o,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    if n.to == "ok" {
        for (id,) in open {
            if let Err(e) = set_page_status(st, &n.org, id, PageAction::Resolve, SYSTEM_ACTOR).await {
                return Outcome::Failed(e.to_string());
            }
        }
        return Outcome::Sent;
    }
    if !open.is_empty() {
        return Outcome::Sent;
    }
    let page = NewPage {
        org: n.org.clone(),
        title: n.title().chars().take(MAX_TITLE).collect(),
        description: n.message.chars().take(MAX_DESCRIPTION).collect(),
        urgency: if n.priority.is_some_and(|p| p >= 3) { "low".into() } else { "high".into() },
        team: Some(team.to_string()),
        policy_id: None,
        source: "monitor",
        incident_pk: None,
        monitor_id: Some(n.monitor_id),
        monitor_group: Some(n.group.clone()),
        created_by: format!("Monitor: {}", n.monitor_name).chars().take(200).collect(),
        created_by_id: None,
    };
    match create_page_row(st, page).await {
        Ok(_) => Outcome::Sent,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

// ================================================================ escalation worker

#[derive(sqlx::FromRow)]
struct DuePage {
    id: i64,
    org_id: String,
    title: String,
    description: String,
    urgency: String,
    team_handle: Option<String>,
    policy_id: Option<String>,
    step: i32,
    round: i32,
}

/// Notifies due escalation steps and auto-resolves pages every 5 seconds.
pub async fn run_escalations(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        if let Err(e) = auto_resolve(&st).await {
            tracing::warn!(error = %e, "on-call auto-resolve failed");
        }
        if let Err(e) = escalation_tick(&st).await {
            tracing::warn!(error = %e, "on-call escalation tick failed");
        }
    }
}

async fn auto_resolve(st: &AppState) -> Result<(), ApiError> {
    let due: Vec<(i64, String)> = sqlx::query_as("SELECT id, org_id FROM on_call_pages WHERE status <> 'resolved' AND resolve_at <= now() LIMIT 100").fetch_all(&st.postgres).await?;
    for (id, org) in due {
        // Another server may have resolved it first.
        let _ = set_page_status(st, &org, id, PageAction::Resolve, SYSTEM_ACTOR).await;
    }
    Ok(())
}

/// The steps a page follows and how often they repeat.
async fn plan(st: &AppState, page: &DuePage) -> Result<(Vec<OnCallStep>, u32), ApiError> {
    if let Some(pid) = &page.policy_id {
        if let Some(p) = policy_row(st, &page.org_id, pid).await? {
            return Ok((p.steps(), p.repeat_count.max(0) as u32));
        }
    }
    let team = page.team_handle.clone().unwrap_or_default();
    Ok((vec![OnCallStep { targets: vec![OnCallTarget { kind: OnCallTargetKind::Team, id: team }], escalate_after_minutes: 1 }], 0))
}

/// The people a step notifies now, in order, once each.
async fn resolve_targets(st: &AppState, org: &str, targets: &[OnCallTarget], at: i64) -> Result<Vec<OnCallUser>, ApiError> {
    let mut ids: Vec<String> = Vec::new();
    for t in targets {
        match t.kind {
            OnCallTargetKind::User => ids.push(t.id.clone()),
            OnCallTargetKind::Team => {
                let members: Vec<(String,)> =
                    sqlx::query_as("SELECT m.user_id::text FROM team_members m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 AND m.handle = $2 AND u.status = 'active' ORDER BY u.name")
                        .bind(org)
                        .bind(&t.id)
                        .fetch_all(&st.postgres)
                        .await?;
                ids.extend(members.into_iter().map(|m| m.0));
            }
            OnCallTargetKind::Schedule => {
                let Ok(row) = schedule_row(st, org, &t.id).await else { continue };
                let ovs = overrides_of(st, org, std::slice::from_ref(&row.id), at).await?.remove(&row.id).unwrap_or_default();
                ids.extend(on_call_at(&row.layers(), &spans(&ovs), row.max(), row.tz(), at));
            }
        }
    }
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    let by_id = users_by_id(st, org, &ids).await?;
    Ok(pick_users(&by_id, &ids))
}

/// A page's notification to one responder (the link opens it in the Pages tab).
fn page_message(public_url: &str, page: &DuePage, to: &OnCallUser) -> Outgoing {
    let (page_id, org, title, description, urgency, team) = (page.id, &page.org_id, &page.title, &page.description, &page.urgency, page.team_handle.as_deref());
    let link = format!("{public_url}/on-call/pages/{page_id}");
    let subject = format!("[On-Call] {title}");
    let team_line = team.map(|t| format!("Team: @{t}\n")).unwrap_or_default();
    Outgoing {
        org: org.to_string(),
        title: subject.clone(),
        email_body: format!("{subject}\n\n{}\n\n{team_line}Urgency: {urgency}\nAcknowledge or resolve: {link}\n", description.trim()),
        slack_text: format!("*{subject}* ({urgency} urgency) for {}\n{}\n<{link}|Acknowledge or resolve>", to.name, description.trim()),
        webhook: json!({
            "event": "on_call.page.triggered",
            "organization": org,
            "page": { "id": page_id, "title": title, "description": description, "urgency": urgency, "team": team, "url": link },
            "responder": { "id": to.id, "name": to.name, "email": to.email },
            "timestamp": now_ms(),
        }),
    }
}

/// The channels a member wants for an urgency ('email' or channel handles).
async fn channels_for(st: &AppState, org: &str, user: &str, urgency: &str) -> Result<Vec<String>, ApiError> {
    let row: Option<(Vec<String>, Vec<String>)> =
        sqlx::query_as("SELECT high_urgency, low_urgency FROM on_call_profiles WHERE org_id = $1 AND user_id::text = $2").bind(org).bind(user).fetch_optional(&st.postgres).await?;
    let chosen = row.map(|(high, low)| if urgency == "low" { low } else { high }).unwrap_or_default();
    Ok(if chosen.is_empty() { vec!["email".into()] } else { chosen })
}

/// Sends to each of the member's channels; returns (channel, error) per failure and the channels reached.
async fn notify_user(st: &AppState, out: &Outgoing, org: &str, user: &OnCallUser, urgency: &str) -> Result<(Vec<String>, Vec<(String, String)>), ApiError> {
    let mut reached = Vec::new();
    let mut failed = Vec::new();
    for ch in channels_for(st, org, &user.id, urgency).await? {
        let recipient = if ch == "email" { user.email.clone() } else { ch.clone() };
        let label = if ch == "email" { "email".to_string() } else { format!("@{ch}") };
        match send(st, &recipient, out).await {
            Outcome::Sent => reached.push(label),
            Outcome::Unknown(e) | Outcome::Failed(e) => failed.push((label, e)),
        }
    }
    Ok((reached, failed))
}

async fn escalation_tick(st: &AppState) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    let due: Vec<DuePage> = sqlx::query_as(
        "SELECT id, org_id, title, description, urgency, team_handle, policy_id::text AS policy_id, step, round FROM on_call_pages \
         WHERE status = 'triggered' AND next_escalation_at <= now() ORDER BY next_escalation_at LIMIT $1 FOR UPDATE SKIP LOCKED",
    )
    .bind(ESCALATION_BATCH)
    .fetch_all(&mut *tx)
    .await?;
    for page in due {
        let (steps, repeat) = plan(st, &page).await?;
        let at = Progress { step: page.step.max(0) as usize, round: page.round.max(0) as u32 };
        let Some(step) = steps.get(at.step) else {
            sqlx::query("UPDATE on_call_pages SET next_escalation_at = NULL WHERE id = $1").bind(page.id).execute(&mut *tx).await?;
            continue;
        };
        if at.step > 0 || at.round > 0 {
            let body = if at.step == 0 { format!("Not acknowledged: starting over (repeat {})", at.round) } else { format!("Not acknowledged: escalated to step {}", at.step + 1) };
            page_event(&mut *tx, page.id, "escalated", SYSTEM_ACTOR, &body).await?;
        }
        let users = resolve_targets(st, &page.org_id, &step.targets, now_ms()).await?;
        if users.is_empty() {
            page_event(&mut *tx, page.id, "failed", SYSTEM_ACTOR, &format!("Step {}: nobody to notify (no one on call or no members)", at.step + 1)).await?;
        }
        for u in &users {
            let out = page_message(&st.config.public_url, &page, u);
            let (reached, failed) = notify_user(st, &out, &page.org_id, u, &page.urgency).await?;
            sqlx::query("INSERT INTO on_call_page_responders (page_id, user_id) VALUES ($1, $2::uuid) ON CONFLICT DO NOTHING").bind(page.id).bind(&u.id).execute(&mut *tx).await?;
            if !reached.is_empty() {
                page_event(&mut *tx, page.id, "notified", SYSTEM_ACTOR, &format!("Notified {} by {}", u.name, reached.join(", "))).await?;
            }
            for (ch, e) in failed {
                page_event(&mut *tx, page.id, "failed", SYSTEM_ACTOR, &format!("Couldn't notify {} by {ch}: {e}", u.name)).await?;
            }
        }
        match after_notifying(&steps, repeat, at) {
            Some((next, wait)) => {
                sqlx::query("UPDATE on_call_pages SET step = $2, round = $3, next_escalation_at = now() + make_interval(mins => $4) WHERE id = $1")
                    .bind(page.id)
                    .bind(next.step as i32)
                    .bind(next.round as i32)
                    .bind(wait)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {
                sqlx::query("UPDATE on_call_pages SET step = $2, next_escalation_at = NULL WHERE id = $1").bind(page.id).bind(at.step as i32 + 1).execute(&mut *tx).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

// ================================================================ profile

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallChannelOption {
    pub handle: String,
    /// email | slack | webhook
    pub kind: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallProfile {
    pub email: String,
    /// 'email' or channel handles, for high-urgency pages.
    pub high_urgency: Vec<String>,
    pub low_urgency: Vec<String>,
    /// Saved at least once.
    pub configured: bool,
    /// What can be chosen: email and the org's Slack and webhook channels.
    pub channels: Vec<OnCallChannelOption>,
    /// Email delivery is configured on this install (OC_SMTP_URL).
    pub email_enabled: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallProfileInput {
    pub high_urgency: Vec<String>,
    pub low_urgency: Vec<String>,
}

async fn channel_options(st: &AppState, org: &str) -> Result<Vec<OnCallChannelOption>, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT handle, kind FROM notification_channels WHERE org_id = $1 ORDER BY handle").bind(org).fetch_all(&st.postgres).await?;
    let mut out = vec![OnCallChannelOption { handle: "email".into(), kind: "email".into() }];
    out.extend(rows.into_iter().map(|(handle, kind)| OnCallChannelOption { handle, kind }));
    Ok(out)
}

#[utoipa::path(get, path = "/api/v1/on-call/profile", operation_id = "get_on_call_profile", responses((status = 200, body = OnCallProfile)), tag = "on-call")]
pub async fn get_profile(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<OnCallProfile> {
    let (email,): (String,) = sqlx::query_as("SELECT email FROM users WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    let row: Option<(Vec<String>, Vec<String>)> =
        sqlx::query_as("SELECT high_urgency, low_urgency FROM on_call_profiles WHERE org_id = $1 AND user_id::text = $2").bind(&ctx.org_id).bind(&ctx.user_id).fetch_optional(&st.postgres).await?;
    let configured = row.is_some();
    let (high_urgency, low_urgency) = row.unwrap_or_else(|| (vec!["email".into()], vec!["email".into()]));
    Ok(Json(OnCallProfile { email, high_urgency, low_urgency, configured, channels: channel_options(&st, &ctx.org_id).await?, email_enabled: st.config.smtp_url.is_some() }))
}

#[utoipa::path(put, path = "/api/v1/on-call/profile", operation_id = "update_on_call_profile", request_body = OnCallProfileInput, responses((status = 200, body = OnCallProfile)), tag = "on-call")]
pub async fn update_profile(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<OnCallProfileInput>) -> ApiResult<OnCallProfile> {
    let options = channel_options(&st, &ctx.org_id).await?;
    let clean = |list: &[String]| -> Result<Vec<String>, ApiError> {
        let mut out: Vec<String> = Vec::new();
        for h in list {
            if !options.iter().any(|o| &o.handle == h) {
                return Err(ApiError::BadRequest(format!("Unknown channel {h}")));
            }
            if !out.contains(h) {
                out.push(h.clone());
            }
        }
        if out.is_empty() {
            return Err(ApiError::BadRequest("Choose at least one way to be notified".into()));
        }
        Ok(out)
    };
    let (high, low) = (clean(&req.high_urgency)?, clean(&req.low_urgency)?);
    sqlx::query(
        "INSERT INTO on_call_profiles (org_id, user_id, high_urgency, low_urgency) VALUES ($1, $2::uuid, $3, $4) \
         ON CONFLICT (org_id, user_id) DO UPDATE SET high_urgency = EXCLUDED.high_urgency, low_urgency = EXCLUDED.low_urgency, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .bind(&high)
    .bind(&low)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "on_call.profile.updated", &ctx.name, &format!("high: {}; low: {}", high.join(", "), low.join(", "))).await;
    get_profile(State(st), ctx).await
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallTestResult {
    pub channel: String,
    /// sent | failed
    pub status: String,
    pub error: Option<String>,
}

/// Sends a test page notification to the member's high-urgency channels.
#[utoipa::path(post, path = "/api/v1/on-call/profile/test", operation_id = "test_on_call_profile", responses((status = 200, body = Vec<OnCallTestResult>)), tag = "on-call")]
pub async fn test_profile(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<OnCallTestResult>> {
    let (email,): (String,) = sqlx::query_as("SELECT email FROM users WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    let me = OnCallUser { id: ctx.user_id.clone(), name: ctx.name.clone(), email };
    let sample = DuePage {
        id: 0,
        org_id: ctx.org_id.clone(),
        title: "Test notification".into(),
        description: "This is how ObserveCraft On-Call reaches you when you're paged.".into(),
        urgency: "high".into(),
        team_handle: None,
        policy_id: None,
        step: 0,
        round: 0,
    };
    let out = page_message(&st.config.public_url, &sample, &me);
    let mut results = Vec::new();
    for ch in channels_for(&st, &ctx.org_id, &ctx.user_id, "high").await? {
        let recipient = if ch == "email" { me.email.clone() } else { ch.clone() };
        let (status, error) = match send(&st, &recipient, &out).await {
            Outcome::Sent => ("sent", None),
            Outcome::Unknown(e) | Outcome::Failed(e) => ("failed", Some(e)),
        };
        results.push(OnCallTestResult { channel: ch, status: status.into(), error });
    }
    ctx.audit(&st, "on_call.profile.test", &ctx.name, &format!("{} channels", results.len())).await;
    Ok(Json(results))
}

// ================================================================ analytics

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallAnalyticsRequest {
    pub range: OnCallRange,
    #[serde(default)]
    pub teams: Vec<String>,
    /// IANA time zone for days and hours (default UTC).
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallDayCount {
    /// Start of the day (Unix ms).
    pub day: i64,
    pub team: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallTeamStats {
    pub team: String,
    pub count: i64,
    pub mtta_seconds: Option<f64>,
    pub mttr_seconds: Option<f64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OnCallSourceCount {
    pub title: String,
    pub source: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnCallAnalytics {
    pub total: i64,
    pub acknowledged: i64,
    pub resolved: i64,
    pub mtta_seconds: Option<f64>,
    pub mttr_seconds: Option<f64>,
    pub per_day: Vec<OnCallDayCount>,
    /// 7 × 24 counts: index weekday (0 = Monday) × 24 + hour.
    pub by_hour: Vec<i64>,
    pub teams: Vec<OnCallTeamStats>,
    /// Pages most often raised with the same title.
    pub top_sources: Vec<OnCallSourceCount>,
}

fn push_analytics_filter(qb: &mut QueryBuilder<Postgres>, org: &str, req: &OnCallAnalyticsRequest) {
    qb.push(" FROM on_call_pages g WHERE g.org_id = ").push_bind(org.to_string());
    qb.push(" AND g.created_at >= to_timestamp(").push_bind(req.range.from as f64).push(" / 1000.0) AND g.created_at < to_timestamp(").push_bind(req.range.to as f64).push(" / 1000.0)");
    if !req.teams.is_empty() {
        qb.push(" AND g.team_handle = ANY(").push_bind(req.teams.clone()).push(")");
    }
}

#[utoipa::path(post, path = "/api/v1/on-call/analytics", operation_id = "on_call_analytics", request_body = OnCallAnalyticsRequest, responses((status = 200, body = OnCallAnalytics)), tag = "on-call")]
pub async fn analytics(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<OnCallAnalyticsRequest>) -> ApiResult<OnCallAnalytics> {
    req.range.check(MAX_ANALYTICS_MS)?;
    let tz = req.time_zone.clone().unwrap_or_else(|| "UTC".into());
    if parse_tz(&tz).is_none() {
        return Err(ApiError::BadRequest(format!("Unknown time zone {tz}")));
    }
    let org = &ctx.org_id;
    let mut q: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT count(*), count(g.acknowledged_at), count(g.resolved_at), avg(extract(epoch FROM g.acknowledged_at - g.created_at))::float8, avg(extract(epoch FROM g.resolved_at - g.created_at))::float8",
    );
    push_analytics_filter(&mut q, org, &req);
    let (total, acknowledged, resolved, mtta_seconds, mttr_seconds): (i64, i64, i64, Option<f64>, Option<f64>) = q.build_query_as().fetch_one(&st.postgres).await?;

    let mut q: QueryBuilder<Postgres> = QueryBuilder::new("SELECT (extract(epoch FROM (date_trunc('day', g.created_at AT TIME ZONE ");
    q.push_bind(tz.clone()).push(") AT TIME ZONE ").push_bind(tz.clone()).push(")) * 1000)::bigint AS day, COALESCE(g.team_handle, '') AS team, count(*) AS count");
    push_analytics_filter(&mut q, org, &req);
    q.push(" GROUP BY 1, 2 ORDER BY 1, 2");
    let per_day: Vec<OnCallDayCount> = q.build_query_as().fetch_all(&st.postgres).await?;

    let mut q: QueryBuilder<Postgres> = QueryBuilder::new("SELECT (extract(isodow FROM g.created_at AT TIME ZONE ");
    q.push_bind(tz.clone()).push(")::int - 1) * 24 + extract(hour FROM g.created_at AT TIME ZONE ").push_bind(tz.clone()).push(")::int AS slot, count(*)");
    push_analytics_filter(&mut q, org, &req);
    q.push(" GROUP BY 1");
    let slots: Vec<(i32, i64)> = q.build_query_as().fetch_all(&st.postgres).await?;
    let mut by_hour = vec![0i64; 168];
    for (slot, count) in slots {
        if let Some(c) = by_hour.get_mut(slot as usize) {
            *c = count;
        }
    }

    let mut q: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT COALESCE(g.team_handle, '') AS team, count(*) AS count, avg(extract(epoch FROM g.acknowledged_at - g.created_at))::float8 AS mtta_seconds, \
         avg(extract(epoch FROM g.resolved_at - g.created_at))::float8 AS mttr_seconds",
    );
    push_analytics_filter(&mut q, org, &req);
    q.push(" GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 50");
    let teams: Vec<OnCallTeamStats> = q.build_query_as().fetch_all(&st.postgres).await?;

    let mut q: QueryBuilder<Postgres> = QueryBuilder::new("SELECT g.title, min(g.source) AS source, count(*) AS count");
    push_analytics_filter(&mut q, org, &req);
    q.push(" GROUP BY g.title ORDER BY 3 DESC, 1 LIMIT 10");
    let top_sources: Vec<OnCallSourceCount> = q.build_query_as().fetch_all(&st.postgres).await?;

    Ok(Json(OnCallAnalytics { total, acknowledged, resolved, mtta_seconds, mttr_seconds, per_day, by_hour, teams, top_sources }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(users: &[&str], start: i64, end: i64) -> OnCallFinalShift {
        OnCallFinalShift { user_ids: users.iter().map(|s| s.to_string()).collect(), start, end, is_override: false }
    }

    #[test]
    fn page_notifications_link_to_the_page() {
        let page = DuePage {
            id: 7,
            org_id: "o".into(),
            title: "Checkout down".into(),
            description: "order-service 5xx".into(),
            urgency: "high".into(),
            team_handle: Some("checkout".into()),
            policy_id: None,
            step: 0,
            round: 0,
        };
        let to = OnCallUser { id: "u1".into(), name: "Ana Example".into(), email: "ana@example.com".into() };
        let out = page_message("https://oc.example.com", &page, &to);
        assert_eq!(out.title, "[On-Call] Checkout down");
        assert!(out.email_body.contains("Team: @checkout\nUrgency: high\nAcknowledge or resolve: https://oc.example.com/on-call/pages/7"));
        assert_eq!(out.slack_text, "*[On-Call] Checkout down* (high urgency) for Ana Example\norder-service 5xx\n<https://oc.example.com/on-call/pages/7|Acknowledge or resolve>");
        assert_eq!(out.webhook["event"], "on_call.page.triggered");
        assert_eq!(out.webhook["page"]["team"], "checkout");
        assert_eq!(out.webhook["responder"]["email"], "ana@example.com");
    }

    #[test]
    fn my_shifts_join_contiguous_stretches() {
        let f = [fin(&["ana"], 0, 10), fin(&["ana", "ben"], 10, 20), fin(&["ben"], 20, 30), fin(&["ana"], 30, 40)];
        assert_eq!(shifts_for("ana", &f), vec![(0, 20), (30, 40)]);
        assert_eq!(shifts_for("ben", &f), vec![(10, 30)]);
        assert!(shifts_for("cy", &f).is_empty());
    }

    #[test]
    fn ranges_and_inputs_are_bounded() {
        assert!(OnCallRange { from: 0, to: 1 }.check(MAX_RANGE_MS).is_ok());
        assert!(OnCallRange { from: 1, to: 1 }.check(MAX_RANGE_MS).is_err());
        assert!(OnCallRange { from: 0, to: MAX_RANGE_MS + 1 }.check(MAX_RANGE_MS).is_err());
        assert_eq!(clean_tags(&["Team:Checkout".into(), "team:checkout".into()]).unwrap(), vec!["team:checkout"]);
        assert!(text("  ", "x", 10).is_err());
        assert_eq!(urgency(None).unwrap(), "high");
        assert!(urgency(Some("urgent")).is_err());
    }
}
