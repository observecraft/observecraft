//! APM's configuration, Datadog-compatible: retention filters
//! (`/api/v2/apm/config/retention-filters[/:id]`, `/api/v2/apm/config/retention-filters-execution-order`,
//! the Terraform provider's `datadog_apm_retention_filter` and `datadog_apm_retention_filter_order`)
//! and span-based metrics (`/api/v2/apm/config/metrics[/:id]`, `datadog_spans_metric`), on Postgres
//! `apm_retention_filters` / `apm_spans_metrics`. APM → Settings reads and writes the same rows
//! (`routes/apm_settings.rs`).
//!
//! - Every org has two default retention filters, rows with fixed ids ([`DEFAULT_FILTERS`]): errors
//!   and App & API Protection traces. They are listed first, can be changed (rate, query, on/off) but
//!   not deleted; new filters can only be `spans-sampling-processor`. A re-order lists every filter.
//! - A span-based metric's id is its name (`-` become `_`, as Datadog stores them); its aggregation
//!   and path can't change. `tag_name` and `include_percentiles` are echoed only when they were sent.
//! - Errors use these endpoints' shape: `{"errors": ["…"]}`.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};

use super::users::require;
use super::DdAuth;
use crate::auth::{Access, AuthContext};
use crate::state::AppState;

pub type DdErr = (StatusCode, Json<Value>);
type DdResult = Result<(StatusCode, Json<Value>), DdErr>;

pub const USER_TYPE: &str = "spans-sampling-processor";
/// (id, name, query, filter type) of the default retention filters, in their execution order.
pub const DEFAULT_FILTERS: [(&str, &str, &str, &str); 2] = [
    ("errors-default", "Error Default", "status:error", "spans-errors-sampling-processor"),
    ("appsec-default", "App and API Protection Default", "@_dd.appsec.enabled:1", "spans-appsec-sampling-processor"),
];

fn errors(status: StatusCode, msgs: &[&str]) -> DdErr {
    (status, Json(json!({ "errors": msgs })))
}

fn db_err(e: sqlx::Error) -> DdErr {
    tracing::error!(error = %e, "APM configuration request failed");
    errors(StatusCode::INTERNAL_SERVER_ERROR, &["internal error"])
}

fn writer(ctx: &AuthContext) -> Result<(), DdErr> {
    require(ctx, Access::Standard).map_err(|_| errors(StatusCode::FORBIDDEN, &["Forbidden"]))
}

fn now_s() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `data` of a JSON:API body, checking its type.
fn data(raw: &Bytes, kind: &str) -> Result<Map<String, Value>, DdErr> {
    let v: Value = serde_json::from_slice(raw).map_err(|_| errors(StatusCode::BAD_REQUEST, &["Invalid JSON body"]))?;
    let d = v.get("data").and_then(Value::as_object).cloned().ok_or_else(|| errors(StatusCode::BAD_REQUEST, &["Field 'data' is required"]))?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != kind) {
        return Err(errors(StatusCode::BAD_REQUEST, &[&format!("Field 'type' is invalid, expected value is '{kind}'")]));
    }
    Ok(d)
}

fn attrs(d: &Map<String, Value>) -> Map<String, Value> {
    d.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default()
}

async fn email(st: &AppState, ctx: &AuthContext) -> String {
    sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id::text = $1 AND org_id = $2")
        .bind(&ctx.user_id)
        .bind(&ctx.org_id)
        .fetch_optional(&st.postgres)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| ctx.name.clone())
}

/// 22 characters of URL-safe base64, as Datadog's retention filter ids.
fn random_id() -> String {
    use base64::Engine;
    let bytes: [u8; 16] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ================================================================ retention filters

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct FilterRow {
    pub id: String,
    pub name: String,
    pub query: String,
    pub filter_type: String,
    pub rate: f64,
    pub trace_rate: f64,
    pub enabled: bool,
    pub execution_order: i32,
    pub created_by: String,
    pub created_at: i64,
    pub modified_by: String,
    pub modified_at: i64,
}

impl FilterRow {
    pub fn is_default(&self) -> bool {
        DEFAULT_FILTERS.iter().any(|d| d.0 == self.id)
    }
}

const FILTER_COLUMNS: &str = "id, name, query, filter_type, rate, trace_rate, enabled, execution_order, created_by, created_at, modified_by, modified_at";

fn installed() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Installs the default retention filters of `org` once per process (what users changed is kept).
pub async fn ensure_defaults(st: &AppState, org: &str) -> Result<(), sqlx::Error> {
    if installed().lock().unwrap().contains(org) {
        return Ok(());
    }
    for (i, (id, name, query, kind)) in DEFAULT_FILTERS.iter().enumerate() {
        sqlx::query("INSERT INTO apm_retention_filters (org_id, id, name, query, filter_type, rate, enabled, execution_order) VALUES ($1, $2, $3, $4, $5, 1, true, $6) ON CONFLICT DO NOTHING")
            .bind(org)
            .bind(id)
            .bind(name)
            .bind(query)
            .bind(kind)
            .bind(i as i32 - 100)
            .execute(&st.postgres)
            .await?;
    }
    installed().lock().unwrap().insert(org.to_string());
    Ok(())
}

pub async fn list_filter_rows(st: &AppState, org: &str) -> Result<Vec<FilterRow>, sqlx::Error> {
    ensure_defaults(st, org).await?;
    sqlx::query_as(&format!("SELECT {FILTER_COLUMNS} FROM apm_retention_filters WHERE org_id = $1 ORDER BY execution_order, created_at, id")).bind(org).fetch_all(&st.postgres).await
}

async fn filter_row(st: &AppState, org: &str, id: &str) -> Result<Option<FilterRow>, sqlx::Error> {
    ensure_defaults(st, org).await?;
    sqlx::query_as(&format!("SELECT {FILTER_COLUMNS} FROM apm_retention_filters WHERE org_id = $1 AND id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await
}

fn not_found(id: &str) -> DdErr {
    errors(StatusCode::NOT_FOUND, &[&format!("retention filter with id: '{id}' not found")])
}

fn rate(a: &Map<String, Value>, key: &str, required: bool) -> Result<Option<f64>, DdErr> {
    match a.get(key) {
        Some(v) if !v.is_null() => match v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())) {
            Some(r) if (0.0..=1.0).contains(&r) => Ok(Some(r)),
            _ => Err(errors(StatusCode::BAD_REQUEST, &["Invalid Pipeline", &format!("'{key}' must exist and be between 0 and 1")])),
        },
        _ if required => Err(errors(StatusCode::BAD_REQUEST, &["Invalid Pipeline", &format!("'{key}' must exist and be between 0 and 1")])),
        _ => Ok(None),
    }
}

fn query_of(a: &Map<String, Value>) -> Option<String> {
    a.get("filter").and_then(|f| f.get("query")).and_then(Value::as_str).map(|q| if q.trim().is_empty() { "*".to_string() } else { q.trim().to_string() })
}

/// Checked attributes of a retention filter: name, query, rate, trace rate, enabled.
pub struct FilterInput {
    pub name: String,
    pub query: String,
    pub rate: f64,
    pub trace_rate: f64,
    pub enabled: bool,
}

fn filter_input(a: &Map<String, Value>, expected_type: &str) -> Result<FilterInput, DdErr> {
    let kind = a.get("filter_type").and_then(Value::as_str).unwrap_or(expected_type);
    if kind != expected_type {
        return Err(errors(StatusCode::BAD_REQUEST, &[&format!("Field 'filter_type' is invalid, expected value is '{expected_type}'")]));
    }
    let name = a.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(errors(StatusCode::BAD_REQUEST, &["Field 'name' is required"]));
    }
    let rate = rate(a, "rate", true)?.unwrap_or(1.0);
    let trace_rate = rate_opt(a)?;
    Ok(FilterInput {
        name: name.chars().take(200).collect(),
        query: query_of(a).unwrap_or_else(|| "*".into()),
        rate,
        trace_rate,
        enabled: a.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    })
}

fn rate_opt(a: &Map<String, Value>) -> Result<f64, DdErr> {
    Ok(rate(a, "trace_rate", false)?.unwrap_or(0.0))
}

pub async fn create_filter_row(st: &AppState, ctx: &AuthContext, a: &Map<String, Value>) -> Result<FilterRow, DdErr> {
    let input = filter_input(a, USER_TYPE)?;
    let rows = list_filter_rows(st, &ctx.org_id).await.map_err(db_err)?;
    if rows.iter().any(|r| r.name == input.name && r.query == input.query && r.rate == input.rate && r.trace_rate == input.trace_rate && r.enabled == input.enabled) {
        // Datadog refuses an exact duplicate of an existing filter.
        return Err(errors(StatusCode::CONFLICT, &["Conflict", "a retention filter with the same name, query and rates already exists"]));
    }
    let order = rows.iter().map(|r| r.execution_order).max().unwrap_or(0).max(0) + 1;
    let (id, by, now) = (random_id(), email(st, ctx).await, now_s());
    sqlx::query(
        "INSERT INTO apm_retention_filters (org_id, id, name, query, filter_type, rate, trace_rate, enabled, execution_order, created_by, created_at, modified_by, modified_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $10, $11)",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.query)
    .bind(USER_TYPE)
    .bind(input.rate)
    .bind(input.trace_rate)
    .bind(input.enabled)
    .bind(order)
    .bind(&by)
    .bind(now)
    .execute(&st.postgres)
    .await
    .map_err(db_err)?;
    filter_row(st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| not_found(&id))
}

pub async fn update_filter_row(st: &AppState, ctx: &AuthContext, id: &str, a: &Map<String, Value>) -> Result<FilterRow, DdErr> {
    let current = filter_row(st, &ctx.org_id, id).await.map_err(db_err)?.ok_or_else(|| not_found(id))?;
    let input = filter_input(a, &current.filter_type)?;
    let (by, now) = (email(st, ctx).await, now_s());
    sqlx::query(
        "UPDATE apm_retention_filters SET name = $3, query = $4, rate = $5, trace_rate = $6, enabled = $7, modified_by = $8, modified_at = $9 WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(&input.name)
    .bind(&input.query)
    .bind(input.rate)
    .bind(input.trace_rate)
    .bind(input.enabled)
    .bind(&by)
    .bind(now)
    .execute(&st.postgres)
    .await
    .map_err(db_err)?;
    filter_row(st, &ctx.org_id, id).await.map_err(db_err)?.ok_or_else(|| not_found(id))
}

pub async fn delete_filter_row(st: &AppState, org: &str, id: &str) -> Result<(), DdErr> {
    let current = filter_row(st, org, id).await.map_err(db_err)?.ok_or_else(|| not_found(id))?;
    if current.is_default() {
        return Err(errors(StatusCode::BAD_REQUEST, &["Default retention filters can't be deleted; disable them instead"]));
    }
    sqlx::query("DELETE FROM apm_retention_filters WHERE org_id = $1 AND id = $2").bind(org).bind(id).execute(&st.postgres).await.map_err(db_err)?;
    Ok(())
}

/// Sets the execution order: `ids` must list every retention filter of the org once.
pub async fn reorder_rows(st: &AppState, org: &str, ids: &[String]) -> Result<(), DdErr> {
    let rows = list_filter_rows(st, org).await.map_err(db_err)?;
    let have: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let sent: HashSet<&str> = ids.iter().map(String::as_str).collect();
    if sent.len() != ids.len() || sent != have {
        return Err(errors(StatusCode::BAD_REQUEST, &["Invalid retention filters order: list every retention filter once"]));
    }
    let mut tx = st.postgres.begin().await.map_err(db_err)?;
    for (i, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE apm_retention_filters SET execution_order = $3 WHERE org_id = $1 AND id = $2").bind(org).bind(id).bind(i as i32).execute(&mut *tx).await.map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

fn filter_doc(r: &FilterRow) -> Value {
    json!({
        "id": r.id,
        "type": "apm_retention_filter",
        "attributes": {
            "name": r.name,
            "rate": r.rate,
            "trace_rate": r.trace_rate,
            "enabled": r.enabled,
            "filter_type": r.filter_type,
            "filter": { "query": r.query },
            "editable": true,
            "execution_order": r.execution_order,
            "modified_by": r.modified_by,
            "modified_at": r.modified_at,
            "created_by": r.created_by,
            "created_at": r.created_at,
        }
    })
}

#[utoipa::path(get, path = "/api/v2/apm/config/retention-filters", operation_id = "datadog_list_apm_retention_filters", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_filters(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DdResult {
    let rows = list_filter_rows(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(json!({ "data": rows.iter().map(filter_doc).collect::<Vec<_>>() }))))
}

#[utoipa::path(get, path = "/api/v2/apm/config/retention-filters/{filter_id}", operation_id = "datadog_get_apm_retention_filter", params(("filter_id" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get_filter(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DdResult {
    let r = filter_row(&st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| not_found(&id))?;
    Ok((StatusCode::OK, Json(json!({ "data": filter_doc(&r) }))))
}

#[utoipa::path(post, path = "/api/v2/apm/config/retention-filters", operation_id = "datadog_create_apm_retention_filter", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 409, body = Object)), tag = "datadog-api")]
pub async fn create_filter(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DdResult {
    writer(&ctx)?;
    let r = create_filter_row(&st, &ctx, &attrs(&data(&raw, "apm_retention_filter")?)).await?;
    Ok((StatusCode::OK, Json(json!({ "data": filter_doc(&r) }))))
}

#[utoipa::path(put, path = "/api/v2/apm/config/retention-filters/{filter_id}", operation_id = "datadog_update_apm_retention_filter", params(("filter_id" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update_filter(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DdResult {
    writer(&ctx)?;
    let d = data(&raw, "apm_retention_filter")?;
    let r = update_filter_row(&st, &ctx, &id, &attrs(&d)).await?;
    Ok((StatusCode::OK, Json(json!({ "data": filter_doc(&r) }))))
}

#[utoipa::path(delete, path = "/api/v2/apm/config/retention-filters/{filter_id}", operation_id = "datadog_delete_apm_retention_filter", params(("filter_id" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete_filter(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DdResult {
    writer(&ctx)?;
    delete_filter_row(&st, &ctx.org_id, &id).await?;
    Ok((StatusCode::OK, Json(json!({}))))
}

#[utoipa::path(put, path = "/api/v2/apm/config/retention-filters-execution-order", operation_id = "datadog_reorder_apm_retention_filters", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object)), tag = "datadog-api")]
pub async fn reorder_filters(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DdResult {
    writer(&ctx)?;
    let v: Value = serde_json::from_slice(&raw).map_err(|_| errors(StatusCode::BAD_REQUEST, &["Invalid JSON body"]))?;
    let ids: Vec<String> = v.get("data").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.get("id").and_then(Value::as_str).map(str::to_string)).collect()).unwrap_or_default();
    reorder_rows(&st, &ctx.org_id, &ids).await?;
    Ok((StatusCode::OK, Json(json!({}))))
}

// ================================================================ span-based metrics

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct MetricRow {
    pub id: String,
    pub query: String,
    pub aggregation_type: String,
    pub path: Option<String>,
    pub include_percentiles: Option<bool>,
    pub group_by: Value,
}

const METRIC_COLUMNS: &str = "id, query, aggregation_type, path, include_percentiles, group_by";

pub async fn list_metric_rows(st: &AppState, org: &str) -> Result<Vec<MetricRow>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {METRIC_COLUMNS} FROM apm_spans_metrics WHERE org_id = $1 ORDER BY id")).bind(org).fetch_all(&st.postgres).await
}

async fn metric_row(st: &AppState, org: &str, id: &str) -> Result<Option<MetricRow>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {METRIC_COLUMNS} FROM apm_spans_metrics WHERE org_id = $1 AND id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await
}

fn metric_not_found(id: &str) -> DdErr {
    errors(StatusCode::NOT_FOUND, &[&format!("not_found(Metric with name '{id}' not found)")])
}

/// A metric name as Datadog stores it: surrounding quotes dropped (the Terraform provider sends the
/// name quoted), `-` and other characters outside `[A-Za-z0-9_.]` become `_`.
pub fn metric_name(raw: &str) -> Result<String, DdErr> {
    let name: String = raw.trim().trim_matches('"').trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).collect();
    if name.is_empty() || name.len() > 200 || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(errors(StatusCode::BAD_REQUEST, &["invalid_argument(Metric name must start with a letter and be at most 200 characters)"]));
    }
    Ok(name)
}

/// `[{"path", "tag_name"?}]`, checked.
fn group_by(a: &Map<String, Value>) -> Result<Option<Value>, DdErr> {
    let Some(v) = a.get("group_by") else { return Ok(None) };
    let mut out = Vec::new();
    for g in v.as_array().map(Vec::as_slice).unwrap_or_default() {
        let path = g.get("path").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if path.is_empty() {
            return Err(errors(StatusCode::BAD_REQUEST, &["invalid_argument(group_by path is required)"]));
        }
        let mut item = json!({ "path": path });
        if let Some(t) = g.get("tag_name").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
            item["tag_name"] = json!(t.trim());
        }
        out.push(item);
    }
    Ok(Some(Value::Array(out)))
}

pub async fn create_metric_row(st: &AppState, ctx: &AuthContext, id: &str, a: &Map<String, Value>) -> Result<MetricRow, DdErr> {
    let name = metric_name(id)?;
    let compute = a.get("compute").and_then(Value::as_object).cloned().unwrap_or_default();
    let kind = compute.get("aggregation_type").and_then(Value::as_str).unwrap_or("");
    let path = compute.get("path").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty());
    match (kind, path) {
        ("count", _) | ("distribution", Some(_)) => {}
        ("distribution", None) => return Err(errors(StatusCode::BAD_REQUEST, &["invalid_argument(a distribution needs the path of a numeric attribute)"])),
        _ => return Err(errors(StatusCode::BAD_REQUEST, &["invalid_argument(aggregation_type must be count or distribution)"])),
    }
    let distribution = kind == "distribution";
    let percentiles = if distribution { compute.get("include_percentiles").and_then(Value::as_bool) } else { None };
    let by = group_by(a)?.unwrap_or_else(|| json!([]));
    let r = sqlx::query(
        "INSERT INTO apm_spans_metrics (org_id, id, query, aggregation_type, path, include_percentiles, group_by, created_by_name) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&ctx.org_id)
    .bind(&name)
    .bind(query_of(a).unwrap_or_else(|| "*".into()))
    .bind(kind)
    .bind(if distribution { path } else { None })
    .bind(percentiles)
    .bind(&by)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await;
    match r {
        Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") => {
            return Err(errors(StatusCode::CONFLICT, &[&format!("conflict(Metric with name '{name}' already exists)")]))
        }
        Err(e) => return Err(db_err(e)),
        Ok(_) => {}
    }
    metric_row(st, &ctx.org_id, &name).await.map_err(db_err)?.ok_or_else(|| metric_not_found(&name))
}

pub async fn update_metric_row(st: &AppState, org: &str, id: &str, a: &Map<String, Value>) -> Result<MetricRow, DdErr> {
    let current = metric_row(st, org, id).await.map_err(db_err)?.ok_or_else(|| metric_not_found(id))?;
    let compute = a.get("compute").and_then(Value::as_object).cloned().unwrap_or_default();
    if compute.get("aggregation_type").and_then(Value::as_str).is_some_and(|k| k != current.aggregation_type)
        || compute.get("path").and_then(Value::as_str).is_some_and(|p| Some(p) != current.path.as_deref())
    {
        return Err(errors(StatusCode::BAD_REQUEST, &["invalid_argument(the aggregation type and path of a metric can't be changed)"]));
    }
    let percentiles = if current.aggregation_type == "distribution" { compute.get("include_percentiles").and_then(Value::as_bool).or(current.include_percentiles) } else { None };
    let query = query_of(a).unwrap_or(current.query);
    let by = group_by(a)?.unwrap_or(current.group_by);
    sqlx::query("UPDATE apm_spans_metrics SET query = $3, include_percentiles = $4, group_by = $5, updated_at = now() WHERE org_id = $1 AND id = $2")
        .bind(org)
        .bind(id)
        .bind(&query)
        .bind(percentiles)
        .bind(&by)
        .execute(&st.postgres)
        .await
        .map_err(db_err)?;
    metric_row(st, org, id).await.map_err(db_err)?.ok_or_else(|| metric_not_found(id))
}

pub async fn delete_metric_row(st: &AppState, org: &str, id: &str) -> Result<(), DdErr> {
    let n = sqlx::query("DELETE FROM apm_spans_metrics WHERE org_id = $1 AND id = $2").bind(org).bind(id).execute(&st.postgres).await.map_err(db_err)?.rows_affected();
    if n == 0 {
        return Err(metric_not_found(id));
    }
    Ok(())
}

fn metric_doc(m: &MetricRow) -> Value {
    let mut compute = json!({ "aggregation_type": m.aggregation_type });
    if let Some(p) = &m.path {
        compute["path"] = json!(p);
    }
    if let Some(p) = m.include_percentiles {
        compute["include_percentiles"] = json!(p);
    }
    json!({
        "id": m.id,
        "type": "spans_metrics",
        "attributes": { "filter": { "query": m.query }, "group_by": m.group_by, "compute": compute }
    })
}

#[utoipa::path(get, path = "/api/v2/apm/config/metrics", operation_id = "datadog_list_spans_metrics", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_metrics(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DdResult {
    let rows = list_metric_rows(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(json!({ "data": rows.iter().map(metric_doc).collect::<Vec<_>>() }))))
}

#[utoipa::path(get, path = "/api/v2/apm/config/metrics/{metric_id}", operation_id = "datadog_get_spans_metric", params(("metric_id" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get_metric(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DdResult {
    let m = metric_row(&st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| metric_not_found(&id))?;
    Ok((StatusCode::OK, Json(json!({ "data": metric_doc(&m) }))))
}

#[utoipa::path(post, path = "/api/v2/apm/config/metrics", operation_id = "datadog_create_spans_metric", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 409, body = Object)), tag = "datadog-api")]
pub async fn create_metric(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DdResult {
    writer(&ctx)?;
    let d = data(&raw, "spans_metrics")?;
    let id = d.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let m = create_metric_row(&st, &ctx, &id, &attrs(&d)).await?;
    Ok((StatusCode::OK, Json(json!({ "data": metric_doc(&m) }))))
}

#[utoipa::path(patch, path = "/api/v2/apm/config/metrics/{metric_id}", operation_id = "datadog_update_spans_metric", params(("metric_id" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update_metric(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DdResult {
    writer(&ctx)?;
    let m = update_metric_row(&st, &ctx.org_id, &id, &attrs(&data(&raw, "spans_metrics")?)).await?;
    Ok((StatusCode::OK, Json(json!({ "data": metric_doc(&m) }))))
}

#[utoipa::path(delete, path = "/api/v2/apm/config/metrics/{metric_id}", operation_id = "datadog_delete_spans_metric", params(("metric_id" = String, Path)), responses((status = 204), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete_metric(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdErr> {
    writer(&ctx)?;
    delete_metric_row(&st, &ctx.org_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_names_are_stored_as_datadog_does() {
        assert_eq!(metric_name("Test-Delete_a_span_based_metric-1681838888").unwrap(), "Test_Delete_a_span_based_metric_1681838888");
        assert_eq!(metric_name("checkout.requests").unwrap(), "checkout.requests");
        assert_eq!(metric_name("\"checkout.requests\"").unwrap(), "checkout.requests");
        assert!(metric_name("9lives").is_err());
        assert!(metric_name("").is_err());
    }

    #[test]
    fn rates_are_between_zero_and_one() {
        let a = |v: Value| v.as_object().cloned().unwrap();
        assert_eq!(rate(&a(json!({"rate": 0.5})), "rate", true).unwrap(), Some(0.5));
        assert_eq!(rate(&a(json!({"rate": "1.0"})), "rate", true).unwrap(), Some(1.0));
        assert!(rate(&a(json!({"rate": 2})), "rate", true).is_err());
        assert!(rate(&a(json!({})), "rate", true).is_err());
        assert_eq!(rate(&a(json!({})), "trace_rate", false).unwrap(), None);
    }

    #[test]
    fn only_user_filters_can_be_created() {
        let a = json!({"name": "x", "rate": 1, "filter_type": "spans-errors-sampling-processor"}).as_object().cloned().unwrap();
        let Err((status, Json(body))) = filter_input(&a, USER_TYPE) else { panic!() };
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["errors"][0], "Field 'filter_type' is invalid, expected value is 'spans-sampling-processor'");
    }

    use super::super::replay::{Feature, TestServer};

    async fn replay(feature: &str, prefix: &str, min: usize) {
        let Some(f) = Feature::load(feature) else { return };
        let Some(s) = TestServer::start(prefix).await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= min, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[tokio::test]
    async fn replays_datadogs_apm_retention_filter_scenarios() {
        replay("v2/apm_retention_filters", "dd-apm-rf-replay", 13).await;
    }

    #[tokio::test]
    async fn replays_datadogs_spans_metrics_scenarios() {
        replay("v2/spans_metrics", "dd-spans-metrics-replay", 5).await;
    }
}
