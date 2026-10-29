//! Datadog-compatible dashboards (`datadog_dashboard`, `datadog_dashboard_json` and the
//! `datadog_dashboard` data source of Datadog's Terraform provider; `DashboardsApi` of the API
//! clients), on the same `dashboards` table as our own API, so a dashboard written here is in the
//! dashboard list and opens in the UI.
//!
//! - `POST /api/v1/dashboard`, `GET`/`PUT`/`DELETE /api/v1/dashboard/{id}`, `GET /api/v1/dashboard`
//!   (`filter[deleted]`, `filter[shared]`, `count`, `start`), bulk `DELETE` and `PATCH` (restore) on
//!   `/api/v1/dashboard`.
//! - `POST /api/unstable/dashboard/widgets/validate`: the provider checks widgets there during `plan`.
//!
//! The Datadog document is stored as received (`dashboards.datadog_document`, widget ids assigned,
//! server-owned fields left out) and read back with the server-owned fields (`id`, `url`,
//! `author_handle`, `author_name`, `created_at`, `modified_at`), so Terraform sees what it sent. Our
//! document (`definition`) is derived from it ([`dashboard_widgets::to_observecraft`]); changes made in
//! the UI since show in the Datadog document on read ([`document`]). Widgets the dashboard page can't
//! render are rejected with their path; nothing is dropped.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use utoipa::ToSchema;

use super::dashboard_widgets::{self as widgets, check_definition, Invalid};
use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::routes::dashboards::new_id;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/dashboard", get(list).post(create).delete(delete_many).patch(restore_many))
        .route("/api/v1/dashboard/:id", get(get_one).put(update).delete(delete_one))
        .route("/api/unstable/dashboard/widgets/validate", post(validate_widgets))
}

/// Fields the server owns: never stored, always filled in on read.
const SERVER_FIELDS: &[&str] = &["id", "url", "author_handle", "author_name", "created_at", "modified_at", "deleted_at"];
/// Our limit on a dashboard's widgets (as our own API).
const MAX_WIDGETS: usize = 500;
const MAX_TAGS: usize = 5;

/// A Datadog dashboard document (Datadog's `Dashboard` schema; other fields are kept as sent).
#[derive(Serialize, Deserialize, ToSchema)]
pub struct DatadogDashboard {
    pub title: String,
    /// `ordered` or `free`.
    pub layout_type: String,
    #[schema(value_type = Vec<Object>)]
    pub widgets: Vec<Value>,
}

/// `GET /api/v1/dashboard`: Datadog's dashboard summaries.
#[derive(Serialize, ToSchema)]
pub struct DatadogDashboardSummaries {
    #[schema(value_type = Vec<Object>)]
    pub dashboards: Vec<Value>,
}

/// `DELETE /api/v1/dashboard/{id}`.
#[derive(Serialize, ToSchema)]
pub struct DatadogDeletedDashboard {
    pub deleted_dashboard_id: String,
}

/// Bulk delete / restore: `{"data": [{"id": "abc-def-ghi", "type": "dashboard"}]}`.
#[derive(Deserialize, ToSchema)]
pub struct DatadogDashboardBulk {
    pub data: Vec<DatadogDashboardRef>,
}

#[derive(Deserialize, ToSchema)]
pub struct DatadogDashboardRef {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

/// `POST /api/unstable/dashboard/widgets/validate`: the widgets to check.
#[derive(Deserialize, ToSchema)]
pub struct DatadogWidgetValidation {
    #[schema(value_type = Vec<Object>)]
    pub widgets: Vec<Value>,
}

/// One result per widget sent, in order.
#[derive(Serialize, ToSchema)]
pub struct DatadogWidgetValidationResults {
    pub results: Vec<DatadogWidgetValidationResult>,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogWidgetValidationResult {
    pub is_valid: bool,
    pub widget_type: Option<String>,
    pub error_message: Option<String>,
    /// Inside the widget's definition, e.g. `requests[0].queries[0].data_source`.
    pub error_path: Option<String>,
}

// ================================================================ documents

/// A dashboard row as the Datadog document is built from it.
#[derive(sqlx::FromRow)]
pub(super) struct Row {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub teams: Vec<String>,
    pub definition: Value,
    pub datadog_document: Option<Value>,
    pub author_handle: Option<String>,
    pub author_name: Option<String>,
    pub created_at: i64,
    pub modified_at: i64,
    pub deleted_at: Option<i64>,
}

const SELECT: &str = "SELECT d.id, d.title, d.description, d.teams, d.definition, d.datadog_document, u.email AS author_handle, u.name AS author_name, \
    (extract(epoch FROM d.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM d.modified_at) * 1000)::bigint AS modified_at, \
    (extract(epoch FROM d.deleted_at) * 1000)::bigint AS deleted_at FROM dashboards d LEFT JOIN users u ON u.id = d.author_id";

/// Datadog's timestamp format (`2026-10-29T10:40:29.941695+00:00`).
pub(super) fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Micros, false)).unwrap_or_default()
}

/// The UI's address of a dashboard (Datadog clients treat `url` as opaque).
pub(super) fn url(id: &str) -> String {
    format!("/dashboards/{id}")
}

fn team_tags(teams: &[String]) -> Vec<String> {
    teams.iter().map(|t| format!("team:{t}")).collect()
}

fn teams_of(doc: &Value) -> Vec<String> {
    doc["tags"].as_array().map(|tags| tags.iter().filter_map(Value::as_str).map(|t| t.strip_prefix("team:").unwrap_or(t).to_string()).collect()).unwrap_or_default()
}

/// The Datadog document of a dashboard: the stored one with what the UI changed since folded in
/// (or, for a dashboard made in the UI, ours translated), plus the server-owned fields.
pub(super) fn document(row: &Row) -> Value {
    let ours = &row.definition;
    let free = ours["layoutType"] == "free";
    let stored = row.datadog_document.as_ref().filter(|s| (s["layout_type"] == "free") == free);
    let mut doc = match row.datadog_document.as_ref() {
        Some(Value::Object(m)) => Value::Object(m.clone()),
        _ => json!({}),
    };
    if row.datadog_document.as_ref().is_none_or(|s| (s["layout_type"] == "free") != free) {
        doc["layout_type"] = json!(if free { "free" } else { "ordered" });
    }
    if doc["title"].as_str() != Some(row.title.as_str()) {
        doc["title"] = json!(row.title);
    }
    if doc["description"].as_str() != row.description.as_deref() {
        doc["description"] = json!(row.description);
    }
    if teams_of(&doc) != row.teams {
        doc["tags"] = json!(team_tags(&row.teams));
    }
    let ours_vars = ours["templateVariables"].as_array().cloned().unwrap_or_default();
    if widgets::template_variables(&doc) != ours_vars {
        doc["template_variables"] = Value::Array(ours_vars.iter().map(|v| template_variable(v, &doc)).collect());
    }
    let (dd_widgets, fixed) = widgets::to_datadog_widgets(ours["widgets"].as_array().map(Vec::as_slice).unwrap_or_default(), stored, free);
    doc["widgets"] = Value::Array(dd_widgets);
    if !free && (fixed || stored.is_none()) && doc["widgets"].as_array().is_some_and(|w| w.iter().any(|w| w.get("layout").is_some())) {
        doc["reflow_type"] = json!("fixed");
    }
    doc["id"] = json!(row.id);
    doc["url"] = json!(url(&row.id));
    doc["author_handle"] = json!(row.author_handle.clone().unwrap_or_default());
    doc["author_name"] = json!(row.author_name);
    doc["created_at"] = json!(iso(row.created_at));
    doc["modified_at"] = json!(iso(row.modified_at));
    doc
}

/// A template variable as Datadog's: the stored one when unchanged, else updated from ours.
fn template_variable(ours: &Value, doc: &Value) -> Value {
    let name = ours["name"].as_str().unwrap_or_default();
    let stored = doc["template_variables"].as_array().and_then(|vars| vars.iter().find(|v| v["name"] == name));
    let Some(stored) = stored else { return json!({ "name": name, "prefix": ours["prefix"], "default": ours["default"] }) };
    let mut v = stored.clone();
    if widgets::template_variables(&json!({ "template_variables": [stored] })).first() == Some(ours) {
        return v;
    }
    v["prefix"] = ours["prefix"].clone();
    if v.get("defaults").is_some() {
        v["defaults"] = json!([ours["default"]]);
    } else {
        v["default"] = ours["default"].clone();
    }
    v
}

fn bad_request(message: impl Into<String>) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, message)
}

fn not_found(id: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, format!("Dashboard with ID {id} not found"))
}

pub(super) fn writer(ctx: &AuthContext) -> Result<(), DdError> {
    if ctx.access < Access::Standard {
        return Err(DdError::new(StatusCode::FORBIDDEN, "Forbidden: your role is read-only"));
    }
    Ok(())
}

pub(super) fn parse_body(body: &Bytes) -> Result<Value, DdError> {
    serde_json::from_slice(body).map_err(|e| bad_request(format!("Invalid JSON body: {e}")))
}

/// Checks a Datadog dashboard document can be stored and rendered.
pub fn validate(doc: &Value) -> Result<(), DdError> {
    let Some(doc) = doc.as_object() else { return Err(bad_request("The dashboard must be a JSON object")) };
    // Datadog accepts an empty title.
    let Some(title) = doc.get("title").and_then(Value::as_str) else { return Err(bad_request("title: required")) };
    if title.chars().count() > 200 {
        return Err(bad_request("title: at most 200 characters"));
    }
    if !matches!(doc.get("layout_type").and_then(Value::as_str), Some("ordered" | "free")) {
        return Err(bad_request("layout_type: must be ordered or free"));
    }
    if let Some(reflow) = doc.get("reflow_type").filter(|r| !r.is_null()) {
        if !matches!(reflow.as_str(), Some("auto" | "fixed")) {
            return Err(bad_request("reflow_type: must be auto or fixed"));
        }
    }
    if let Some(description) = doc.get("description").filter(|d| !d.is_null()) {
        if !description.is_string() {
            return Err(bad_request("description: must be a string"));
        }
    }
    let Some(list) = doc.get("widgets").and_then(Value::as_array) else { return Err(bad_request("widgets: a list of widgets is required")) };
    if list.len() > MAX_WIDGETS {
        return Err(bad_request(format!("widgets: a dashboard may hold at most {MAX_WIDGETS} widgets")));
    }
    for (i, w) in list.iter().enumerate() {
        check_definition(&w["definition"]).map_err(|e: Invalid| bad_request(e.under(&format!("widgets[{i}].definition")).to_string()))?;
    }
    if let Some(tags) = doc.get("tags").filter(|t| !t.is_null()) {
        let tags = tags.as_array().ok_or_else(|| bad_request("tags: must be a list"))?;
        if tags.len() > MAX_TAGS {
            return Err(bad_request(format!("tags: at most {MAX_TAGS} tags")));
        }
        if !tags.iter().all(|t| t.as_str().and_then(|t| t.strip_prefix("team:")).is_some_and(|team| !team.is_empty())) {
            return Err(bad_request("tags: only team tags (team:<name>) are supported"));
        }
    }
    for (i, v) in items(doc.get("template_variables")).iter().enumerate() {
        if !v["name"].is_string() {
            return Err(bad_request(format!("template_variables[{i}].name: required")));
        }
        if !v["default"].is_null() && !v["defaults"].is_null() {
            return Err(bad_request(format!("template_variables[{i}]: set default or defaults, not both")));
        }
        if items(v.get("defaults")).iter().any(|d| d.as_str().is_none_or(str::is_empty)) {
            return Err(bad_request(format!("template_variables[{i}].defaults: values must be non-empty strings")));
        }
    }
    for (i, preset) in items(doc.get("template_variable_presets")).iter().enumerate() {
        for (j, v) in items(preset.get("template_variables")).iter().enumerate() {
            if !v["value"].is_null() && !v["values"].is_null() {
                return Err(bad_request(format!("template_variable_presets[{i}].template_variables[{j}]: set value or values, not both")));
            }
        }
    }
    Ok(())
}

fn items(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// The document as stored: without the server-owned fields, every widget with an id.
fn prepare(doc: Value) -> Value {
    let mut map: Map<String, Value> = match doc {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    for f in SERVER_FIELDS {
        map.remove(*f);
    }
    let mut doc = Value::Object(map);
    if let Some(list) = doc["widgets"].as_array_mut() {
        widgets::assign_widget_ids(list);
    }
    doc
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE d.org_id = $1 AND d.id = $2 AND d.deleted_at IS NULL")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.ok_or_else(|| not_found(id))
}

// ================================================================ handlers

/// Creates a dashboard from a Datadog dashboard document (Datadog-compatible).
#[utoipa::path(post, path = "/api/v1/dashboard", operation_id = "datadog_create_dashboard", request_body = DatadogDashboard,
    responses((status = 200, body = DatadogDashboard), (status = 400, body = super::DatadogErrors), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let doc = parse_body(&body)?;
    validate(&doc)?;
    let doc = prepare(doc);
    let id = new_id();
    sqlx::query("INSERT INTO dashboards (id, org_id, title, description, kind, author_id, teams, definition, datadog_document) VALUES ($1, $2, $3, $4, 'custom', $5::uuid, $6, $7, $8)")
        .bind(&id)
        .bind(&ctx.org_id)
        .bind(doc["title"].as_str())
        .bind(doc["description"].as_str())
        .bind(&ctx.user_id)
        .bind(teams_of(&doc))
        .bind(widgets::to_observecraft(&doc))
        .bind(&doc)
        .execute(&st.postgres)
        .await?;
    Ok(Json(document(&load(&st, &ctx.org_id, &id).await?)))
}

/// A dashboard as a Datadog dashboard document (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/dashboard/{id}", operation_id = "datadog_get_dashboard", params(("id" = String, Path)),
    responses((status = 200, body = DatadogDashboard), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<Value>, DdError> {
    Ok(Json(document(&load(&st, &ctx.org_id, &id).await?)))
}

/// Replaces a dashboard with a Datadog dashboard document (Datadog-compatible).
#[utoipa::path(put, path = "/api/v1/dashboard/{id}", operation_id = "datadog_update_dashboard", params(("id" = String, Path)), request_body = DatadogDashboard,
    responses((status = 200, body = DatadogDashboard), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let doc = parse_body(&body)?;
    validate(&doc)?;
    let doc = prepare(doc);
    let done = sqlx::query(
        "UPDATE dashboards SET title = $3, description = $4, teams = $5, definition = $6, datadog_document = $7, modified_at = now() \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(doc["title"].as_str())
    .bind(doc["description"].as_str())
    .bind(teams_of(&doc))
    .bind(widgets::to_observecraft(&doc))
    .bind(&doc)
    .execute(&st.postgres)
    .await?;
    if done.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    Ok(Json(document(&load(&st, &ctx.org_id, &id).await?)))
}

/// Deletes a dashboard (restorable, as in our dashboard list's Deleted view; Datadog-compatible).
#[utoipa::path(delete, path = "/api/v1/dashboard/{id}", operation_id = "datadog_delete_dashboard", params(("id" = String, Path)),
    responses((status = 200, body = DatadogDeletedDashboard), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<DatadogDeletedDashboard>, DdError> {
    writer(&ctx)?;
    let done = sqlx::query("UPDATE dashboards SET deleted_at = now() WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    Ok(Json(DatadogDeletedDashboard { deleted_dashboard_id: id }))
}

fn bulk_ids(body: &Bytes) -> Result<Vec<String>, DdError> {
    let req: DatadogDashboardBulk = serde_json::from_slice(body).map_err(|e| bad_request(format!("Invalid body: {e}")))?;
    if req.data.is_empty() {
        return Err(bad_request("data: at least one dashboard is required"));
    }
    if let Some(r) = req.data.iter().find(|r| r.kind.as_deref().is_some_and(|k| k != "dashboard")) {
        return Err(bad_request(format!("data: {} has type {:?}, expected dashboard", r.id, r.kind.as_deref().unwrap_or_default())));
    }
    Ok(req.data.into_iter().map(|r| r.id).collect())
}

/// Ids of `ids` that aren't dashboards of the org (`deleted`: in that state).
async fn missing(st: &AppState, org: &str, ids: &[String], deleted: Option<bool>) -> Result<Vec<String>, DdError> {
    let found: Vec<(String,)> = sqlx::query_as("SELECT id FROM dashboards WHERE org_id = $1 AND id = ANY($2) AND ($3::boolean IS NULL OR (deleted_at IS NOT NULL) = $3)")
        .bind(org)
        .bind(ids)
        .bind(deleted)
        .fetch_all(&st.postgres)
        .await?;
    Ok(ids.iter().filter(|id| !found.iter().any(|(f,)| f == *id)).cloned().collect())
}

/// Deletes several dashboards (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v1/dashboard", operation_id = "datadog_delete_dashboards", request_body = DatadogDashboardBulk,
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_many(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<StatusCode, DdError> {
    writer(&ctx)?;
    let ids = bulk_ids(&body)?;
    let absent = missing(&st, &ctx.org_id, &ids, Some(false)).await?;
    if !absent.is_empty() {
        return Err(DdError::new(StatusCode::NOT_FOUND, format!("Dashboards not found: {}", absent.join(", "))));
    }
    sqlx::query("UPDATE dashboards SET deleted_at = now() WHERE org_id = $1 AND id = ANY($2) AND deleted_at IS NULL").bind(&ctx.org_id).bind(&ids).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Restores deleted dashboards (Datadog-compatible).
#[utoipa::path(patch, path = "/api/v1/dashboard", operation_id = "datadog_restore_dashboards", request_body = DatadogDashboardBulk,
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn restore_many(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<StatusCode, DdError> {
    writer(&ctx)?;
    let ids = bulk_ids(&body)?;
    let absent = missing(&st, &ctx.org_id, &ids, None).await?;
    if !absent.is_empty() {
        return Err(DdError::new(StatusCode::NOT_FOUND, format!("Dashboards not found: {}", absent.join(", "))));
    }
    sqlx::query("UPDATE dashboards SET deleted_at = NULL WHERE org_id = $1 AND id = ANY($2)").bind(&ctx.org_id).bind(&ids).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The org's dashboards as Datadog summaries (Datadog-compatible). `filter[deleted]=true` lists
/// deleted ones; `filter[shared]=true` lists none (sharing isn't part of this API); `count` and
/// `start` page.
#[utoipa::path(get, path = "/api/v1/dashboard", operation_id = "datadog_list_dashboards",
    params(("filter[shared]" = Option<bool>, Query), ("filter[deleted]" = Option<bool>, Query), ("count" = Option<i64>, Query), ("start" = Option<i64>, Query)),
    responses((status = 200, body = DatadogDashboardSummaries), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> Result<Json<DatadogDashboardSummaries>, DdError> {
    let flag = |name: &str| -> Result<bool, DdError> {
        match q.get(name).map(String::as_str) {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(other) => Err(bad_request(format!("{name}: expected true or false, got {other:?}"))),
        }
    };
    let number = |name: &str| -> Result<Option<i64>, DdError> { q.get(name).map(|v| v.parse::<i64>().map_err(|_| bad_request(format!("{name}: expected an integer")))).transpose() };
    let (shared, deleted, count, start) = (flag("filter[shared]")?, flag("filter[deleted]")?, number("count")?, number("start")?);
    if shared {
        return Ok(Json(DatadogDashboardSummaries { dashboards: vec![] }));
    }
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE d.org_id = $1 AND (d.deleted_at IS NOT NULL) = $2 ORDER BY d.created_at, d.id LIMIT $3 OFFSET $4"))
        .bind(&ctx.org_id)
        .bind(deleted)
        .bind(count.map(|c| c.clamp(0, 10_000)))
        .bind(start.unwrap_or(0).max(0))
        .fetch_all(&st.postgres)
        .await?;
    let dashboards = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "title": r.title, "description": r.description,
                "layout_type": if r.definition["layoutType"] == "free" { "free" } else { "ordered" },
                "url": url(&r.id), "is_read_only": false, "created_at": iso(r.created_at), "modified_at": iso(r.modified_at),
                "author_handle": r.author_handle.clone().unwrap_or_default(), "deleted_at": r.deleted_at.map(iso),
            })
        })
        .collect();
    Ok(Json(DatadogDashboardSummaries { dashboards }))
}

/// Checks widget definitions the way a dashboard write does, one result per widget (the provider
/// calls it during `plan`; Datadog-compatible).
#[utoipa::path(post, path = "/api/unstable/dashboard/widgets/validate", operation_id = "datadog_validate_dashboard_widgets", request_body = DatadogWidgetValidation,
    responses((status = 200, body = DatadogWidgetValidationResults), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn validate_widgets(_: DdAuth, body: Bytes) -> Result<Json<DatadogWidgetValidationResults>, DdError> {
    let req: DatadogWidgetValidation = serde_json::from_slice(&body).map_err(|e| bad_request(format!("Invalid body: {e}")))?;
    let results = req
        .widgets
        .iter()
        .map(|w| {
            let widget_type = w["definition"]["type"].as_str().map(str::to_string);
            match check_definition(&w["definition"]) {
                Ok(()) => DatadogWidgetValidationResult { is_valid: true, widget_type, error_message: None, error_path: None },
                Err(e) => DatadogWidgetValidationResult { is_valid: false, widget_type, error_message: Some(e.message), error_path: Some(e.path) },
            }
        })
        .collect();
    Ok(Json(DatadogWidgetValidationResults { results }))
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::replay::{new_org_with_roles, user_with_app_key, Feature, Interaction, TestServer};
    use super::*;

    /// Response fields Datadog adds although they weren't sent; we return only what was sent (so
    /// `datadog_dashboard_json` plans stay clean) and don't compare them.
    pub fn ignored() -> Vec<String> {
        vec!["restricted_roles".into(), "is_read_only".into()]
    }

    /// Replaces widget definitions we don't render by a metric timeseries, in requests and recorded
    /// answers alike: for scenarios whose subject isn't the widget (CRUD, lists, template variables)
    /// but whose fixture dashboards Datadog's client builds with widgets we don't support.
    pub fn adapt(interactions: &[Interaction]) -> Vec<Interaction> {
        fn walk(v: &mut Value) {
            match v {
                Value::Object(m) => {
                    if let Some(def) = m.get_mut("definition") {
                        if def.get("type").is_some() && check_definition(def).is_err() {
                            *def = json!({ "type": "timeseries", "requests": [{ "q": "avg:system.cpu.user{*}" }] });
                        }
                    }
                    m.values_mut().for_each(walk);
                }
                Value::Array(a) => a.iter_mut().for_each(walk),
                _ => {}
            }
        }
        let fix = |text: &str| match serde_json::from_str::<Value>(text) {
            Ok(mut v) => {
                walk(&mut v);
                v.to_string()
            }
            Err(_) => text.to_string(),
        };
        interactions
            .iter()
            .cloned()
            .map(|mut it| {
                it.request.body = fix(&it.request.body);
                it.response.body = fix(&it.response.body);
                it
            })
            .collect()
    }

    /// Scenarios replayed as recorded.
    const AS_RECORDED: &[&str] = &[
        "Create a new dashboard with a formulas and functions change widget",
        "Create a new dashboard with a formulas and functions treemap widget",
        "Create a new dashboard with a live default_timeframe returns \"OK\" response",
        "Create a new dashboard with a query value widget using the percentile aggregator",
        "Create a new dashboard with a query value widget using timeseries background",
        "Create a new dashboard with a query_value widget containing a description",
        "Create a new dashboard with a timeseries widget and an overlay request",
        "Create a new dashboard with a timeseries widget using formulas and functions metrics query with combined semantic_mode",
        "Create a new dashboard with a timeseries widget using formulas and functions metrics query with native semantic_mode",
        "Create a new dashboard with a toplist widget sorted by group",
        "Create a new dashboard with a toplist widget with stacked type and no legend specified",
        "Create a new dashboard with check_status widget",
        "Create a new dashboard with event_stream list_stream widget",
        "Create a new dashboard with formula and function heatmap widget",
        "Create a new dashboard with formulas and functions events query using facet group by",
        "Create a new dashboard with formulas and functions events query using flat group by fields",
        "Create a new dashboard with free_text widget",
        "Create a new dashboard with heatmap widget",
        "Create a new dashboard with image widget",
        "Create a new dashboard with invalid team tags returns \"Bad Request\" response",
        "Create a new dashboard with list_stream widget with a valid sort parameter ASC",
        "Create a new dashboard with list_stream widget with a valid sort parameter DESC",
        "Create a new dashboard with log_stream widget",
        "Create a new dashboard with logs query table widget and storage parameter",
        "Create a new dashboard with manage_status widget",
        "Create a new dashboard with manage_status widget and show_priority parameter",
        "Create a new dashboard with note widget",
        "Create a new dashboard with query_table widget",
        "Create a new dashboard with query_table widget and cell_display_mode is trend",
        "Create a new dashboard with query_table widget and text formatting",
        "Create a new dashboard with query_value widget",
        "Create a new dashboard with sunburst widget and metrics data",
        "Create a new dashboard with team tags returns \"OK\" response",
        "Create a new dashboard with timeseries widget and formula style attributes",
        "Create a new dashboard with timeseries widget containing style attributes",
        "Create a new dashboard with timeseries widget using has_value_labels",
        "Create a new dashboard with timeseries widget using order_by tags",
        "Create a new dashboard with timeseries widget using order_by values",
        "Create a new dashboard with timeseries widget with custom_unit",
        "Create a new dashboard with timeseries widget without order_by for backward compatibility",
        "Create a new dashboard with toplist widget",
        "Get all dashboards returns \"OK\" response with pagination",
    ];

    /// Scenarios about the dashboard rather than a widget, replayed with [`adapt`]ed fixtures.
    const ADAPTED: &[&str] = &[
        "Create a new dashboard with template variable defaults and default returns \"Bad Request\" response",
        "Create a new dashboard with template variable defaults returns \"OK\" response",
        "Create a new dashboard with template variable defaults whose value has no length returns \"Bad Request\" response",
        "Create a new dashboard with template variable presets using values and value returns \"Bad Request\" response",
        "Create a new dashboard with template variable presets using values returns \"OK\" response",
        "Create a new dashboard with template variable type field returns \"OK\" response",
        "Create a new dashboard returns \"OK\" response",
        "Delete a dashboard returns \"OK\" response",
        "Delete dashboards returns \"No Content\" response",
        "Get a dashboard returns \"OK\" response",
        "Get all dashboards returns \"OK\" response",
        "Get deleted dashboards returns \"OK\" response",
        "Restore deleted dashboards returns \"No Content\" response",
        "Update a dashboard returns \"OK\" response",
        "Update a dashboard with tags returns \"OK\" response",
    ];

    fn is_dashboard_write(it: &Interaction) -> bool {
        matches!(it.request.method.as_str(), "POST" | "PUT") && url::Url::parse(&it.request.url).is_ok_and(|u| u.path().starts_with("/api/v1/dashboard") && !u.path().contains("/lists"))
    }

    #[tokio::test]
    async fn replays_the_dashboards_feature() {
        let Some(s) = TestServer::start("dd-dashboards").await else { return };
        let feature = Feature::load("v1/dashboards");
        for name in AS_RECORDED.iter().chain(ADAPTED) {
            assert!(feature.cassette(feature.scenario(name)).is_some(), "{name}: no cassette");
        }
        let mut failures = vec![];
        let mut refused = vec![];
        for scenario in &feature.scenarios {
            let Some(interactions) = feature.cassette(scenario) else { continue };
            let name = scenario.name.as_str();
            let mut r = s.replayer();
            r.ignore = ignored();
            if AS_RECORDED.contains(&name) || ADAPTED.contains(&name) {
                let interactions = if ADAPTED.contains(&name) { adapt(&interactions) } else { interactions };
                let (_, problems) = r.replay_interactions(feature.sends(scenario), name, &interactions).await;
                failures.extend(problems);
                continue;
            }
            // Every other scenario's dashboard has a widget we don't render: its write is refused,
            // naming the widget.
            let Some(write) = interactions.iter().find(|it| is_dashboard_write(it)) else {
                failures.push(format!("{name}: no dashboard write to refuse"));
                continue;
            };
            let (exchanges, _) = r.replay_interactions(feature.sends(scenario), name, std::slice::from_ref(write)).await;
            let reason = exchanges[0].body["errors"][0].as_str().unwrap_or_default().to_string();
            if exchanges[0].status != 400 || !reason.starts_with("widgets[") || !reason.contains("not supported") {
                failures.push(format!("{name}: expected a 400 naming the unsupported widget, got {} {}", exchanges[0].status, exchanges[0].body));
            }
            refused.push(format!("{name} | {reason}"));
        }
        // The refused scenarios and why (listed in docs/datadog-api.md).
        eprintln!("refused:\n{}", refused.join("\n"));
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// One request with the Datadog key header (`dd`) or our Bearer authentication (`!dd`).
    pub async fn call(method: &str, url: &str, key: &str, dd: bool, body: Option<Value>) -> (u16, Value) {
        let mut req = reqwest::Client::new().request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), url);
        req = if dd { req.header("DD-APPLICATION-KEY", key) } else { req.bearer_auth(key) };
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.expect("request");
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// What the Terraform provider sends for an ordered dashboard with template variables and a
    /// timeseries, query value, toplist, note and group.
    pub fn terraform_dashboard() -> Value {
        json!({
            "title": "Checkout service", "description": "Managed by Terraform", "layout_type": "ordered", "reflow_type": "fixed",
            "notify_list": [], "tags": ["team:checkout"],
            "template_variables": [{ "name": "env", "prefix": "env", "available_values": [], "defaults": ["prod"] }, { "name": "service", "prefix": "service", "defaults": ["checkout-api"] }],
            "template_variable_presets": [{ "name": "staging", "template_variables": [{ "name": "env", "values": ["staging"] }] }],
            "widgets": [
                { "definition": { "type": "timeseries", "title": "Requests", "show_legend": true, "requests": [{ "display_type": "line", "q": "sum:trace.http.request.hits{$env,$service}.as_rate()" }] }, "layout": { "x": 0, "y": 0, "width": 6, "height": 3 } },
                { "definition": { "type": "query_value", "title": "p95 latency", "precision": 2, "autoscale": true, "requests": [{ "response_format": "scalar",
                    "queries": [{ "data_source": "metrics", "name": "query1", "query": "p95:trace.http.request{$env,$service}", "aggregator": "avg" }], "formulas": [{ "formula": "query1" }],
                    "conditional_formats": [{ "comparator": ">", "value": 0.5, "palette": "white_on_red" }] }] }, "layout": { "x": 6, "y": 0, "width": 3, "height": 3 } },
                { "definition": { "type": "note", "content": "Runbook: see the on-call page", "background_color": "yellow", "font_size": "14", "text_align": "left", "show_tick": false }, "layout": { "x": 9, "y": 0, "width": 3, "height": 3 } },
                { "definition": { "type": "group", "title": "Hosts", "layout_type": "ordered", "background_color": "vivid_blue", "widgets": [
                    { "definition": { "type": "toplist", "title": "CPU by host", "requests": [{ "q": "top(avg:system.cpu.user{$env} by {host}, 10, 'mean', 'desc')" }] }, "layout": { "x": 0, "y": 0, "width": 12, "height": 3 } },
                ] }, "layout": { "x": 0, "y": 3, "width": 12, "height": 4 } },
            ],
        })
    }

    /// The document without the fields the server adds.
    fn sent_part(mut doc: Value) -> Value {
        for f in SERVER_FIELDS {
            doc.as_object_mut().unwrap().remove(*f);
        }
        fn strip_ids(widgets: &mut Value) {
            for w in widgets.as_array_mut().into_iter().flatten() {
                w.as_object_mut().unwrap().remove("id");
                if let Some(children) = w["definition"].get_mut("widgets") {
                    strip_ids(children);
                }
            }
        }
        strip_ids(&mut doc["widgets"]);
        doc
    }

    #[tokio::test]
    async fn terraform_dashboards_read_back_as_written_and_open_in_the_ui() {
        let Some(s) = TestServer::start("dd-dash-crud").await else { return };
        let api = |p: &str| format!("{}{p}", s.base);
        let (status, created) = call("POST", &api("/api/v1/dashboard"), &s.app_key, true, Some(terraform_dashboard())).await;
        assert_eq!(status, 200, "{created}");
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(id.len(), 11, "abc-def-ghi: {id}");
        assert_eq!(sent_part(created.clone()), terraform_dashboard());
        assert_eq!(created["url"], json!(format!("/dashboards/{id}")));
        assert!(created["author_handle"].as_str().unwrap().ends_with("@example.com"));
        assert_eq!(created["author_name"], "Terraform Test");
        assert!(created["created_at"].as_str().unwrap().ends_with("+00:00"));
        assert!(created["widgets"][3]["definition"]["widgets"][0]["id"].is_i64());
        let (status, read) = call("GET", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, None).await;
        assert_eq!((status, &read), (200, &created));

        // Our API and UI see the same dashboard.
        let (status, ours) = call("GET", &api(&format!("/api/v1/dashboards/{id}")), &s.app_key, false, None).await;
        assert_eq!(status, 200, "{ours}");
        assert_eq!((ours["title"].as_str(), ours["kind"].as_str(), ours["layoutType"].as_str()), (Some("Checkout service"), Some("custom"), Some("grid")));
        assert_eq!(ours["teams"], json!(["checkout"]));
        assert_eq!(ours["templateVariables"], json!([{ "name": "env", "prefix": "env", "default": "prod" }, { "name": "service", "prefix": "service", "default": "checkout-api" }]));
        let w = &ours["widgets"];
        assert_eq!(w[0]["definition"]["type"], "integration");
        assert_eq!(w[0]["definition"]["original"], terraform_dashboard()["widgets"][0]["definition"]);
        assert_eq!(w[2]["definition"]["type"], "note");
        assert_eq!(w[3]["definition"]["widgets"][0]["definition"]["original"]["type"], "toplist");
        let (_, page) = call(
            "POST",
            &api("/api/v1/dashboards/search"),
            &s.app_key,
            false,
            Some(json!({ "query": "Checkout", "preset": "all", "sort": { "field": "title", "dir": "asc" }, "offset": 0, "limit": 10 })),
        )
        .await;
        assert_eq!(page["dashboards"][0]["id"], json!(id), "{page}");

        // Edits in the UI show in the Datadog document; untouched widgets stay as written.
        let mut widgets = w.as_array().unwrap().clone();
        widgets[0]["layout"]["w"] = json!(12);
        widgets.remove(2);
        let (status, _) = call("PATCH", &api(&format!("/api/v1/dashboards/{id}")), &s.app_key, false, Some(json!({ "title": "Checkout", "widgets": widgets }))).await;
        assert_eq!(status, 200);
        let (_, read) = call("GET", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, None).await;
        assert_eq!(read["title"], "Checkout");
        let dd = read["widgets"].as_array().unwrap();
        assert_eq!(dd.len(), 3);
        assert_eq!(dd[0]["layout"], json!({ "x": 0, "y": 0, "width": 12, "height": 3 }));
        assert_eq!(dd[0]["definition"], created["widgets"][0]["definition"]);
        assert_eq!((&dd[1], &dd[2]), (&created["widgets"][1], &created["widgets"][3]));
        assert_eq!(read["template_variables"], created["template_variables"]);
        assert_eq!(read["template_variable_presets"], created["template_variable_presets"]);

        // A Terraform update replaces the document.
        let mut update = terraform_dashboard();
        update["title"] = json!("Checkout service v2");
        update["widgets"].as_array_mut().unwrap().truncate(2);
        update["template_variables"][0]["defaults"] = json!(["staging"]);
        let (status, updated) = call("PUT", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, Some(update.clone())).await;
        assert_eq!(status, 200, "{updated}");
        assert_eq!(sent_part(updated.clone()), update);
        let (_, ours) = call("GET", &api(&format!("/api/v1/dashboards/{id}")), &s.app_key, false, None).await;
        assert_eq!((ours["title"].as_str(), ours["widgets"].as_array().map(Vec::len)), (Some("Checkout service v2"), Some(2)));
        assert_eq!(ours["templateVariables"][0]["default"], "staging");

        // The data source lists dashboards by title.
        let (_, all) = call("GET", &api("/api/v1/dashboard"), &s.app_key, true, None).await;
        let summary = all["dashboards"].as_array().unwrap().iter().find(|d| d["id"] == json!(id)).cloned().unwrap();
        assert_eq!((summary["title"].as_str(), summary["layout_type"].as_str(), summary["deleted_at"].is_null()), (Some("Checkout service v2"), Some("ordered"), true));
        assert_eq!(call("GET", &api("/api/v1/dashboard?filter[shared]=true"), &s.app_key, true, None).await.1, json!({ "dashboards": [] }));
        let (_, page) = call("GET", &api("/api/v1/dashboard?count=1&start=0"), &s.app_key, true, None).await;
        assert_eq!(page["dashboards"].as_array().map(Vec::len), Some(1));

        // Delete: gone for Terraform, restorable.
        assert_eq!(call("DELETE", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, None).await, (200, json!({ "deleted_dashboard_id": id })));
        assert_eq!(call("GET", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, None).await.0, 404);
        assert_eq!(call("DELETE", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, None).await.0, 404);
        let (_, deleted) = call("GET", &api("/api/v1/dashboard?filter[deleted]=true"), &s.app_key, true, None).await;
        assert!(deleted["dashboards"][0]["deleted_at"].is_string(), "{deleted}");
        let bulk = json!({ "data": [{ "id": id, "type": "dashboard" }] });
        assert_eq!(call("PATCH", &api("/api/v1/dashboard"), &s.app_key, true, Some(bulk.clone())).await.0, 204);
        assert_eq!(call("GET", &api(&format!("/api/v1/dashboard/{id}")), &s.app_key, true, None).await.0, 200);
        assert_eq!(call("DELETE", &api("/api/v1/dashboard"), &s.app_key, true, Some(bulk.clone())).await.0, 204);
        assert_eq!(call("DELETE", &api("/api/v1/dashboard"), &s.app_key, true, Some(bulk)).await.0, 404);
    }

    #[tokio::test]
    async fn dashboards_we_cant_render_are_refused_with_the_widget_path() {
        let Some(s) = TestServer::start("dd-dash-refuse").await else { return };
        let mut doc = terraform_dashboard();
        doc["widgets"][3]["definition"]["widgets"].as_array_mut().unwrap().push(json!({ "definition": { "type": "hostmap", "requests": { "fill": { "q": "avg:system.cpu.user{*}" } } } }));
        let (status, body) = call("POST", &format!("{}/api/v1/dashboard", s.base), &s.app_key, true, Some(doc)).await;
        assert_eq!(status, 400);
        assert!(body["errors"][0].as_str().unwrap().starts_with("widgets[3].definition.widgets[1].definition.type: widget type \"hostmap\" is not supported"), "{body}");
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM dashboards WHERE org_id = $1").bind(&s.org).fetch_one(&s.st.postgres).await.unwrap();
        assert_eq!(count, 0);
        for (doc, message) in [
            (json!({ "title": "x", "widgets": [] }), "layout_type: must be ordered or free"),
            (json!({ "title": "x", "layout_type": "ordered" }), "widgets: a list of widgets is required"),
            (json!({ "title": "x", "layout_type": "ordered", "widgets": [], "tags": ["env:prod"] }), "tags: only team tags (team:<name>) are supported"),
        ] {
            assert_eq!(call("POST", &format!("{}/api/v1/dashboard", s.base), &s.app_key, true, Some(doc)).await, (400, json!({ "errors": [message] })));
        }

        // What the provider checks during plan: one result per widget, the path inside the definition.
        let widgets = json!({ "layout_type": "ordered", "widgets": [
            { "definition": { "type": "timeseries", "requests": [{ "q": "avg:system.cpu.user{*}" }] } },
            { "definition": { "type": "timeseries", "requests": [{ "queries": [{ "data_source": "cloud_cost", "name": "query1", "query": "sum:aws.cost.amortized{*}" }] }] } },
            { "definition": { "type": "group", "layout_type": "ordered", "widgets": [] } },
        ] });
        let (status, body) = call("POST", &format!("{}/api/unstable/dashboard/widgets/validate", s.base), &s.app_key, true, Some(widgets)).await;
        assert_eq!(status, 200);
        assert_eq!(body["results"][0], json!({ "is_valid": true, "widget_type": "timeseries", "error_message": null, "error_path": null }));
        assert_eq!((body["results"][1]["is_valid"].as_bool(), body["results"][1]["error_path"].as_str()), (Some(false), Some("requests[0].queries[0].data_source")));
        assert_eq!(body["results"][2]["is_valid"], true);
    }

    #[tokio::test]
    async fn dashboards_belong_to_the_keys_org_and_follow_its_role() {
        let Some(s) = TestServer::start("dd-dash-tenancy").await else { return };
        let (_, created) = call("POST", &format!("{}/api/v1/dashboard", s.base), &s.app_key, true, Some(terraform_dashboard())).await;
        let url = format!("{}/api/v1/dashboard/{}", s.base, created["id"].as_str().unwrap());
        let other = new_org_with_roles(&s.st, "dd-dash-other").await;
        let (_, other_key) = user_with_app_key(&s.st, &other, "admin").await;
        assert_eq!(call("GET", &url, &other_key, true, None).await.0, 404);
        assert_eq!(call("PUT", &url, &other_key, true, Some(terraform_dashboard())).await.0, 404);
        assert_eq!(call("DELETE", &url, &other_key, true, None).await.0, 404);
        let (_, listed) = call("GET", &format!("{}/api/v1/dashboard", s.base), &other_key, true, None).await;
        assert_eq!(listed, json!({ "dashboards": [] }));

        let (_, reader) = user_with_app_key(&s.st, &s.org, "readonly").await;
        assert_eq!(call("GET", &url, &reader, true, None).await.0, 200);
        assert_eq!(call("POST", &format!("{}/api/v1/dashboard", s.base), &reader, true, Some(terraform_dashboard())).await.0, 403);
        assert_eq!(call("PUT", &url, &reader, true, Some(terraform_dashboard())).await.0, 403);
        assert_eq!(call("DELETE", &url, &reader, true, None).await.0, 403);
        assert_eq!(call("GET", &url, "ocak_not-a-key", true, None).await.0, 403);
    }

    #[tokio::test]
    async fn dashboards_made_in_the_ui_read_as_datadog_documents() {
        let Some(s) = TestServer::start("dd-dash-ui").await else { return };
        let (status, made) = call("POST", &format!("{}/api/v1/dashboards", s.base), &s.app_key, false, Some(json!({ "title": "Made in the UI", "teams": ["storefront"], "layoutType": "grid" }))).await;
        assert_eq!(status, 200, "{made}");
        let id = made["id"].as_str().unwrap();
        let widgets = json!([{ "id": "w1", "definition": { "type": "timeseries", "title": "Orders", "requests": [{ "query": "sum:orders.created{service:order-service}" }], "display": "bars" }, "layout": { "x": 0, "y": 0, "w": 6, "h": 4 } }]);
        call("PATCH", &format!("{}/api/v1/dashboards/{id}", s.base), &s.app_key, false, Some(json!({ "widgets": widgets }))).await;
        let (status, doc) = call("GET", &format!("{}/api/v1/dashboard/{id}", s.base), &s.app_key, true, None).await;
        assert_eq!(status, 200);
        assert_eq!((doc["title"].as_str(), doc["layout_type"].as_str(), doc["reflow_type"].as_str()), (Some("Made in the UI"), Some("ordered"), Some("fixed")));
        assert_eq!(doc["tags"], json!(["team:storefront"]));
        assert_eq!(doc["widgets"][0]["definition"]["requests"][0]["queries"][0]["query"], "sum:orders.created{service:order-service}");
        assert_eq!(doc["widgets"][0]["layout"], json!({ "x": 0, "y": 0, "width": 6, "height": 2 }));
        // What it reads is a document it can write back unchanged.
        let (status, written) = call("PUT", &format!("{}/api/v1/dashboard/{id}", s.base), &s.app_key, true, Some(doc.clone())).await;
        assert_eq!(status, 200, "{written}");
        assert_eq!(sent_part(written), sent_part(doc));
    }
}
