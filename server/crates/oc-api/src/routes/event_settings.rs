//! Event Management → Settings: correlators, maintenance windows and work item attributes (custom
//! fields of the Triage Inbox's work items, and their values). How the inbox applies correlators and
//! windows is in `event_correlation`.

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::event_correlation::{Correlator, MaintenanceWindow};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_CORRELATORS: i64 = 50;
const MAX_WINDOWS: i64 = 200;
const MAX_ATTRIBUTES: i64 = 30;
const MAX_QUERY_CHARS: usize = 1_000;
const MAX_GROUP_BY: usize = 5;
/// Group-by keys that aren't tags.
const CORE_KEYS: [&str; 4] = ["source", "host", "status", "title"];

fn tag_key_ok(k: &str) -> bool {
    !k.is_empty() && k.len() <= 100 && k.chars().all(|c| c.is_ascii_alphanumeric() || "_.-/".contains(c))
}

fn name_ok(n: &str) -> Result<(), String> {
    if n.is_empty() || n.chars().count() > 200 {
        return Err("The name is 1 to 200 characters".into());
    }
    Ok(())
}

fn query_ok(q: &str) -> Result<(), String> {
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("The query is at most {MAX_QUERY_CHARS} characters"));
    }
    Ok(())
}

fn unique(e: sqlx::Error, what: &str, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A {what} named '{name}' already exists")),
        _ => e.into(),
    }
}

// ================================================================ correlators

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventCorrelator {
    pub id: String,
    pub name: String,
    /// Event search: which warning and error events it groups ('' = all).
    pub query: String,
    /// `source`, `host`, `status`, `title` or tag keys: one work item per distinct values.
    pub group_by: Vec<String>,
    /// Events further apart than this start a new work item.
    pub window_minutes: i32,
    pub enabled: bool,
    /// Correlators are tried in this order.
    pub position: i32,
    pub updated_by: String,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventCorrelatorInput {
    pub name: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub group_by: Vec<String>,
    pub window_minutes: i32,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub position: i32,
}

fn yes() -> bool {
    true
}

fn validate_correlator(i: &mut EventCorrelatorInput) -> Result<(), String> {
    i.name = i.name.trim().to_string();
    i.query = i.query.trim().to_string();
    let mut keys: Vec<String> = Vec::new();
    for k in i.group_by.iter().map(|k| k.trim()).filter(|k| !k.is_empty()) {
        if !CORE_KEYS.contains(&k) && !tag_key_ok(k) {
            return Err(format!("Can't group by '{k}': use source, host, status, title or a tag key"));
        }
        if !keys.iter().any(|x| x == k) {
            keys.push(k.to_string());
        }
    }
    i.group_by = keys;
    name_ok(&i.name)?;
    query_ok(&i.query)?;
    if i.group_by.len() > MAX_GROUP_BY {
        return Err(format!("Group by at most {MAX_GROUP_BY} keys"));
    }
    if !(1..=1440).contains(&i.window_minutes) {
        return Err("The time window is 1 minute to 24 hours".into());
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct CorrelatorRow {
    id: String,
    name: String,
    query: String,
    group_by: Vec<String>,
    window_minutes: i32,
    enabled: bool,
    position: i32,
    updated_by: String,
    updated_at: i64,
}

impl From<CorrelatorRow> for EventCorrelator {
    fn from(r: CorrelatorRow) -> Self {
        EventCorrelator {
            id: r.id,
            name: r.name,
            query: r.query,
            group_by: r.group_by,
            window_minutes: r.window_minutes,
            enabled: r.enabled,
            position: r.position,
            updated_by: r.updated_by,
            updated_at: r.updated_at,
        }
    }
}

const CORRELATOR_COLS: &str = "id::text AS id, name, query, group_by, window_minutes, enabled, position, updated_by, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

async fn correlator_rows(st: &AppState, org: &str) -> Result<Vec<CorrelatorRow>, ApiError> {
    Ok(sqlx::query_as(&format!("SELECT {CORRELATOR_COLS} FROM event_correlators WHERE org_id = $1 ORDER BY position, lower(name)")).bind(org).fetch_all(&st.postgres).await?)
}

/// The org's enabled correlators, in order (the inbox reads them on every request).
pub(crate) async fn active_correlators(st: &AppState, org: &str) -> Result<Vec<Correlator>, ApiError> {
    Ok(correlator_rows(st, org)
        .await?
        .into_iter()
        .filter(|r| r.enabled)
        .map(|r| Correlator { id: r.id, name: r.name, query: r.query, group_by: r.group_by, window_ms: i64::from(r.window_minutes) * 60_000 })
        .collect())
}

async fn get_correlator(st: &AppState, org: &str, id: &str) -> Result<EventCorrelator, ApiError> {
    let row: Option<CorrelatorRow> =
        sqlx::query_as(&format!("SELECT {CORRELATOR_COLS} FROM event_correlators WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(EventCorrelator::from).ok_or_else(|| ApiError::NotFound("Correlator not found".into()))
}

#[utoipa::path(get, path = "/api/v1/events/correlators", operation_id = "list_event_correlators", responses((status = 200, body = Vec<EventCorrelator>)), tag = "events")]
pub async fn list_correlators(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<EventCorrelator>> {
    Ok(Json(correlator_rows(&st, &ctx.org_id).await?.into_iter().map(EventCorrelator::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/events/correlators", operation_id = "create_event_correlator", request_body = EventCorrelatorInput, responses((status = 200, body = EventCorrelator)), tag = "events")]
pub async fn create_correlator(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<EventCorrelatorInput>) -> ApiResult<EventCorrelator> {
    validate_correlator(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM event_correlators WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_CORRELATORS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_CORRELATORS} correlators")));
    }
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO event_correlators (org_id, name, query, group_by, window_minutes, enabled, position, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&input.name)
            .bind(&input.query)
            .bind(&input.group_by)
            .bind(input.window_minutes)
            .bind(input.enabled)
            .bind(input.position)
            .bind(&ctx.name)
            .fetch_one(&st.postgres)
            .await
            .map_err(|e| unique(e, "correlator", &input.name))?;
    ctx.audit(&st, "event_correlator.created", &input.name, &input.query).await;
    Ok(Json(get_correlator(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/events/correlators/{id}", operation_id = "update_event_correlator", params(("id" = String, Path)), request_body = EventCorrelatorInput, responses((status = 200, body = EventCorrelator)), tag = "events")]
pub async fn update_correlator(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<EventCorrelatorInput>) -> ApiResult<EventCorrelator> {
    validate_correlator(&mut input).map_err(ApiError::BadRequest)?;
    let done = sqlx::query(
        "UPDATE event_correlators SET name = $3, query = $4, group_by = $5, window_minutes = $6, enabled = $7, position = $8, updated_by = $9, updated_at = now() WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.query)
    .bind(&input.group_by)
    .bind(input.window_minutes)
    .bind(input.enabled)
    .bind(input.position)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await
    .map_err(|e| unique(e, "correlator", &input.name))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Correlator not found".into()));
    }
    ctx.audit(&st, "event_correlator.updated", &input.name, &input.query).await;
    Ok(Json(get_correlator(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/events/correlators/{id}", operation_id = "delete_event_correlator", params(("id" = String, Path)), responses((status = 204)), tag = "events")]
pub async fn delete_correlator(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM event_correlators WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Correlator not found".into())) };
    ctx.audit(&st, "event_correlator.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ maintenance windows

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventMaintenanceWindow {
    pub id: String,
    pub name: String,
    /// Event search: which events are expected during the window ('' = all).
    pub query: String,
    /// Epoch milliseconds.
    pub starts_at: i64,
    pub ends_at: i64,
    pub created_by: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventMaintenanceWindowInput {
    pub name: String,
    #[serde(default)]
    pub query: String,
    pub starts_at: i64,
    pub ends_at: i64,
}

fn validate_window(i: &mut EventMaintenanceWindowInput) -> Result<(), String> {
    i.name = i.name.trim().to_string();
    i.query = i.query.trim().to_string();
    name_ok(&i.name)?;
    query_ok(&i.query)?;
    if i.ends_at <= i.starts_at {
        return Err("The window ends after it starts".into());
    }
    if i.ends_at - i.starts_at > 31 * 86_400_000 {
        return Err("A maintenance window is at most 31 days".into());
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct WindowRow {
    id: String,
    name: String,
    query: String,
    starts_at: i64,
    ends_at: i64,
    created_by: String,
}

const WINDOW_COLS: &str = "id::text AS id, name, query, (extract(epoch FROM starts_at) * 1000)::bigint AS starts_at, (extract(epoch FROM ends_at) * 1000)::bigint AS ends_at, created_by";

impl From<WindowRow> for EventMaintenanceWindow {
    fn from(r: WindowRow) -> Self {
        EventMaintenanceWindow { id: r.id, name: r.name, query: r.query, starts_at: r.starts_at, ends_at: r.ends_at, created_by: r.created_by }
    }
}

/// The org's maintenance windows overlapping [from, to].
pub(crate) async fn windows_between(st: &AppState, org: &str, from: i64, to: i64) -> Result<Vec<MaintenanceWindow>, ApiError> {
    let rows: Vec<WindowRow> = sqlx::query_as(&format!(
        "SELECT {WINDOW_COLS} FROM event_maintenance_windows WHERE org_id = $1 AND ends_at >= to_timestamp($2::bigint / 1000.0) AND starts_at <= to_timestamp($3::bigint / 1000.0) ORDER BY starts_at"
    ))
    .bind(org)
    .bind(from)
    .bind(to)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().map(|r| MaintenanceWindow { name: r.name, query: r.query, from: r.starts_at, to: r.ends_at }).collect())
}

#[utoipa::path(get, path = "/api/v1/events/maintenance-windows", operation_id = "list_event_maintenance_windows", responses((status = 200, body = Vec<EventMaintenanceWindow>)), tag = "events")]
pub async fn list_windows(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<EventMaintenanceWindow>> {
    let rows: Vec<WindowRow> =
        sqlx::query_as(&format!("SELECT {WINDOW_COLS} FROM event_maintenance_windows WHERE org_id = $1 ORDER BY starts_at DESC")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(EventMaintenanceWindow::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/events/maintenance-windows", operation_id = "create_event_maintenance_window", request_body = EventMaintenanceWindowInput, responses((status = 200, body = EventMaintenanceWindow)), tag = "events")]
pub async fn create_window(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<EventMaintenanceWindowInput>) -> ApiResult<EventMaintenanceWindow> {
    validate_window(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM event_maintenance_windows WHERE org_id = $1 AND ends_at > now()").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_WINDOWS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_WINDOWS} current maintenance windows")));
    }
    let row: WindowRow = sqlx::query_as(&format!(
        "INSERT INTO event_maintenance_windows (org_id, name, query, starts_at, ends_at, created_by) VALUES ($1, $2, $3, to_timestamp($4::bigint / 1000.0), to_timestamp($5::bigint / 1000.0), $6) RETURNING {WINDOW_COLS}"
    ))
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(&input.query)
    .bind(input.starts_at)
    .bind(input.ends_at)
    .bind(&ctx.name)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "event_maintenance_window.created", &input.name, &input.query).await;
    Ok(Json(row.into()))
}

#[utoipa::path(delete, path = "/api/v1/events/maintenance-windows/{id}", operation_id = "delete_event_maintenance_window", params(("id" = String, Path)), responses((status = 204)), tag = "events")]
pub async fn delete_window(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("DELETE FROM event_maintenance_windows WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Maintenance window not found".into())) };
    ctx.audit(&st, "event_maintenance_window.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ work item attributes

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkItemAttributeKind {
    /// One of `options`.
    Select,
    /// Free text.
    Text,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemAttribute {
    pub id: String,
    pub name: String,
    pub kind: WorkItemAttributeKind,
    pub options: Vec<String>,
    pub position: i32,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemAttributeInput {
    pub name: String,
    pub kind: WorkItemAttributeKind,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub position: i32,
}

fn validate_attribute(i: &mut WorkItemAttributeInput) -> Result<(), String> {
    i.name = i.name.trim().to_string();
    name_ok(&i.name)?;
    let mut options: Vec<String> = Vec::new();
    for o in i.options.iter().map(|o| o.trim()).filter(|o| !o.is_empty()) {
        if o.chars().count() > 100 {
            return Err("Values are at most 100 characters".into());
        }
        if !options.iter().any(|x| x == o) {
            options.push(o.to_string());
        }
    }
    i.options = options;
    match i.kind {
        WorkItemAttributeKind::Select if i.options.is_empty() => Err("A select lists at least one value".into()),
        WorkItemAttributeKind::Select if i.options.len() > 50 => Err("A select has at most 50 values".into()),
        WorkItemAttributeKind::Text => {
            i.options.clear();
            Ok(())
        }
        _ => Ok(()),
    }
}

#[derive(sqlx::FromRow)]
struct AttributeRow {
    id: String,
    name: String,
    kind: String,
    options: Vec<String>,
    position: i32,
}

impl From<AttributeRow> for WorkItemAttribute {
    fn from(r: AttributeRow) -> Self {
        WorkItemAttribute { id: r.id, name: r.name, kind: if r.kind == "select" { WorkItemAttributeKind::Select } else { WorkItemAttributeKind::Text }, options: r.options, position: r.position }
    }
}

const ATTRIBUTE_COLS: &str = "id::text AS id, name, kind, options, position";

async fn get_attribute(st: &AppState, org: &str, id: &str) -> Result<WorkItemAttribute, ApiError> {
    let row: Option<AttributeRow> =
        sqlx::query_as(&format!("SELECT {ATTRIBUTE_COLS} FROM event_work_item_attributes WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(WorkItemAttribute::from).ok_or_else(|| ApiError::NotFound("Attribute not found".into()))
}

fn kind_str(k: WorkItemAttributeKind) -> &'static str {
    match k {
        WorkItemAttributeKind::Select => "select",
        WorkItemAttributeKind::Text => "text",
    }
}

#[utoipa::path(get, path = "/api/v1/events/work-item-attributes", operation_id = "list_work_item_attributes", responses((status = 200, body = Vec<WorkItemAttribute>)), tag = "events")]
pub async fn list_attributes(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<WorkItemAttribute>> {
    let rows: Vec<AttributeRow> =
        sqlx::query_as(&format!("SELECT {ATTRIBUTE_COLS} FROM event_work_item_attributes WHERE org_id = $1 ORDER BY position, lower(name)")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(WorkItemAttribute::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/events/work-item-attributes", operation_id = "create_work_item_attribute", request_body = WorkItemAttributeInput, responses((status = 200, body = WorkItemAttribute)), tag = "events")]
pub async fn create_attribute(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<WorkItemAttributeInput>) -> ApiResult<WorkItemAttribute> {
    validate_attribute(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM event_work_item_attributes WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_ATTRIBUTES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_ATTRIBUTES} work item attributes")));
    }
    let (id,): (String,) = sqlx::query_as("INSERT INTO event_work_item_attributes (org_id, name, kind, options, position) VALUES ($1, $2, $3, $4, $5) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&input.name)
        .bind(kind_str(input.kind))
        .bind(&input.options)
        .bind(input.position)
        .fetch_one(&st.postgres)
        .await
        .map_err(|e| unique(e, "attribute", &input.name))?;
    ctx.audit(&st, "work_item_attribute.created", &input.name, kind_str(input.kind)).await;
    Ok(Json(get_attribute(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/events/work-item-attributes/{id}", operation_id = "update_work_item_attribute", params(("id" = String, Path)), request_body = WorkItemAttributeInput, responses((status = 200, body = WorkItemAttribute)), tag = "events")]
pub async fn update_attribute(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<WorkItemAttributeInput>) -> ApiResult<WorkItemAttribute> {
    validate_attribute(&mut input).map_err(ApiError::BadRequest)?;
    let done = sqlx::query("UPDATE event_work_item_attributes SET name = $3, kind = $4, options = $5, position = $6 WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(kind_str(input.kind))
        .bind(&input.options)
        .bind(input.position)
        .execute(&st.postgres)
        .await
        .map_err(|e| unique(e, "attribute", &input.name))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Attribute not found".into()));
    }
    ctx.audit(&st, "work_item_attribute.updated", &input.name, kind_str(input.kind)).await;
    Ok(Json(get_attribute(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/events/work-item-attributes/{id}", operation_id = "delete_work_item_attribute", params(("id" = String, Path)), responses((status = 204)), tag = "events")]
pub async fn delete_attribute(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("DELETE FROM event_work_item_attributes WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Attribute not found".into())) };
    ctx.audit(&st, "work_item_attribute.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemAttributeValue {
    pub attribute_id: String,
    /// '' clears it.
    pub value: String,
}

/// Sets one attribute of a work item (a select's value must be one of its options).
#[utoipa::path(put, path = "/api/v1/events/work-items/{id}/attributes", operation_id = "set_work_item_attribute", params(("id" = String, Path)), request_body = WorkItemAttributeValue, responses((status = 204)), tag = "events")]
pub async fn set_value(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<WorkItemAttributeValue>) -> Result<StatusCode, ApiError> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("invalid work item id".into()));
    }
    let attribute = get_attribute(&st, &ctx.org_id, &input.attribute_id).await?;
    let value = input.value.trim();
    if value.chars().count() > 500 {
        return Err(ApiError::BadRequest("Values are at most 500 characters".into()));
    }
    if attribute.kind == WorkItemAttributeKind::Select && !value.is_empty() && !attribute.options.iter().any(|o| o == value) {
        return Err(ApiError::BadRequest(format!("'{value}' isn't a value of {}", attribute.name)));
    }
    if value.is_empty() {
        sqlx::query("DELETE FROM event_work_item_values WHERE org_id = $1 AND work_item_id = $2 AND attribute_id::text = $3")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(&attribute.id)
            .execute(&st.postgres)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO event_work_item_values (org_id, work_item_id, attribute_id, value, updated_by) VALUES ($1, $2, $3::uuid, $4, $5) \
             ON CONFLICT (org_id, work_item_id, attribute_id) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = now()",
        )
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&attribute.id)
        .bind(value)
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?;
    }
    ctx.audit(&st, "work_item.attribute_set", &id, &format!("{} = {value}", attribute.name)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Attribute values (attribute id → value) of the work items `ids`.
pub(crate) async fn values_of(st: &AppState, org: &str, ids: &[String]) -> Result<HashMap<String, BTreeMap<String, String>>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT work_item_id, attribute_id::text, value FROM event_work_item_values WHERE org_id = $1 AND work_item_id = ANY($2)").bind(org).bind(ids).fetch_all(&st.postgres).await?;
    let mut out: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    for (item, attribute, value) in rows {
        out.entry(item).or_default().insert(attribute, value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlators_are_checked() {
        let mut ok = EventCorrelatorInput {
            name: " Shop pods ".into(),
            query: " source:kubernetes ".into(),
            group_by: vec![" kube_namespace ".into(), "source".into(), "source".into(), "".into()],
            window_minutes: 30,
            enabled: true,
            position: 0,
        };
        assert_eq!(validate_correlator(&mut ok), Ok(()));
        assert_eq!((ok.name.as_str(), ok.query.as_str(), ok.group_by.clone()), ("Shop pods", "source:kubernetes", vec!["kube_namespace".to_string(), "source".to_string()]));
        let bad = |f: &dyn Fn(&mut EventCorrelatorInput)| {
            let mut i = EventCorrelatorInput { name: "x".into(), query: String::new(), group_by: vec![], window_minutes: 10, enabled: true, position: 0 };
            f(&mut i);
            validate_correlator(&mut i).is_err()
        };
        assert!(bad(&|i| i.name = " ".into()));
        assert!(bad(&|i| i.window_minutes = 0));
        assert!(bad(&|i| i.window_minutes = 1441));
        assert!(bad(&|i| i.group_by = vec!["bad key".into()]));
        assert!(bad(&|i| i.group_by = (0..6).map(|n| format!("k{n}")).collect()));
        assert!(bad(&|i| i.query = "a".repeat(MAX_QUERY_CHARS + 1)));
    }

    #[test]
    fn windows_and_attributes_are_checked() {
        let mut w = EventMaintenanceWindowInput { name: " Upgrade ".into(), query: "kube_namespace:shop".into(), starts_at: 1_000, ends_at: 2_000 };
        assert_eq!(validate_window(&mut w), Ok(()));
        assert!(validate_window(&mut EventMaintenanceWindowInput { name: "x".into(), query: String::new(), starts_at: 2_000, ends_at: 2_000 }).is_err());
        assert!(validate_window(&mut EventMaintenanceWindowInput { name: "x".into(), query: String::new(), starts_at: 0, ends_at: 32 * 86_400_000 }).is_err());

        let mut select = WorkItemAttributeInput { name: "Severity".into(), kind: WorkItemAttributeKind::Select, options: vec![" SEV-1 ".into(), "SEV-2".into(), "SEV-1".into()], position: 0 };
        assert_eq!(validate_attribute(&mut select), Ok(()));
        assert_eq!(select.options, vec!["SEV-1", "SEV-2"]);
        assert!(validate_attribute(&mut WorkItemAttributeInput { name: "Severity".into(), kind: WorkItemAttributeKind::Select, options: vec![], position: 0 }).is_err());
        let mut text = WorkItemAttributeInput { name: "Ticket".into(), kind: WorkItemAttributeKind::Text, options: vec!["x".into()], position: 0 };
        assert_eq!(validate_attribute(&mut text), Ok(()));
        assert!(text.options.is_empty(), "free text has no values");
    }
}
