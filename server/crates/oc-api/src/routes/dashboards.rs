//! Dashboards, dashboard lists and the monitor summary widget, stored in Postgres.
//!
//! A dashboard row holds metadata in columns and the document the UI edits (layout type, template
//! variables, widgets) in `definition`. Widgets are opaque JSON here: the web app owns their schema
//! (`web/src/api/dashboards/types.ts`). Per-user state (starred, last viewed) lives in
//! `dashboard_user_state`. Every query is scoped by org.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};

use super::monitor_engine::MonitorStatus;
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CurrentUser, OrgId};

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DashboardKind {
    Custom,
    Integration,
}

/// grid = 12-column grid; auto = uniform flow; free = fine grid, overlap allowed.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LayoutType {
    Grid,
    Auto,
    Free,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TemplateVariable {
    pub name: String,
    /// Tag key the variable filters on.
    pub prefix: String,
    pub default: String,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct UserRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: DashboardKind,
    pub logo: Option<String>,
    pub author: Option<UserRef>,
    pub teams: Vec<String>,
    /// 0..5, from how often the dashboard is opened.
    pub popularity: u8,
    pub created_at: i64,
    pub modified_at: i64,
    pub viewed_at: Option<i64>,
    pub starred: bool,
    pub deleted_at: Option<i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: DashboardKind,
    pub logo: Option<String>,
    pub author: Option<UserRef>,
    pub teams: Vec<String>,
    pub popularity: u8,
    pub created_at: i64,
    pub modified_at: i64,
    pub viewed_at: Option<i64>,
    pub starred: bool,
    pub deleted_at: Option<i64>,
    pub layout_type: LayoutType,
    pub template_variables: Vec<TemplateVariable>,
    /// Widget documents (schema owned by the web app).
    #[schema(value_type = Vec<Object>)]
    pub widgets: Vec<Value>,
}

/// Row shape shared by summaries and full dashboards (definition only selected for the latter).
#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    title: String,
    description: Option<String>,
    kind: String,
    logo: Option<String>,
    author_id: Option<String>,
    author_name: Option<String>,
    teams: Vec<String>,
    view_count: i32,
    created_at: i64,
    modified_at: i64,
    viewed_at: Option<i64>,
    starred: bool,
    deleted_at: Option<i64>,
    definition: Option<Value>,
    total: Option<i64>,
}

const SELECT: &str = "SELECT d.id, d.title, d.description, d.kind, d.logo, u.id::text AS author_id, u.name AS author_name, d.teams, d.view_count, \
    (extract(epoch FROM d.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM d.modified_at) * 1000)::bigint AS modified_at, \
    (extract(epoch FROM s.viewed_at) * 1000)::bigint AS viewed_at, COALESCE(s.starred, false) AS starred, \
    (extract(epoch FROM d.deleted_at) * 1000)::bigint AS deleted_at";
const FROM: &str = " FROM dashboards d LEFT JOIN users u ON u.id = d.author_id \
    LEFT JOIN dashboard_user_state s ON s.dashboard_id = d.id AND s.user_id = ";

pub(super) fn popularity(views: i32) -> u8 {
    // 0 views → 0, then one step per doubling, capped at 5.
    (32 - (views.max(0) as u32).leading_zeros()).min(5) as u8
}

impl Row {
    fn summary(self) -> DashboardSummary {
        DashboardSummary {
            author: self.author_id.map(|id| UserRef {
                id,
                name: self.author_name.unwrap_or_default(),
            }),
            kind: if self.kind == "integration" {
                DashboardKind::Integration
            } else {
                DashboardKind::Custom
            },
            popularity: popularity(self.view_count),
            id: self.id,
            title: self.title,
            description: self.description,
            logo: self.logo,
            teams: self.teams,
            created_at: self.created_at,
            modified_at: self.modified_at,
            viewed_at: self.viewed_at,
            starred: self.starred,
            deleted_at: self.deleted_at,
        }
    }

    fn dashboard(mut self) -> Dashboard {
        let def = self.definition.take().unwrap_or(Value::Null);
        let layout_type =
            serde_json::from_value(def["layoutType"].clone()).unwrap_or(LayoutType::Grid);
        let template_variables =
            serde_json::from_value(def["templateVariables"].clone()).unwrap_or_default();
        let widgets = serde_json::from_value(def["widgets"].clone()).unwrap_or_default();
        let s = self.summary();
        Dashboard {
            id: s.id,
            title: s.title,
            description: s.description,
            kind: s.kind,
            logo: s.logo,
            author: s.author,
            teams: s.teams,
            popularity: s.popularity,
            created_at: s.created_at,
            modified_at: s.modified_at,
            viewed_at: s.viewed_at,
            starred: s.starred,
            deleted_at: s.deleted_at,
            layout_type,
            template_variables,
            widgets,
        }
    }
}

async fn load(st: &AppState, org: &str, user: &str, id: &str) -> Result<Dashboard, ApiError> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(SELECT);
    qb.push(", d.definition, NULL::bigint AS total")
        .push(FROM)
        .push_bind(user)
        .push("::uuid WHERE d.org_id = ")
        .push_bind(org)
        .push(" AND d.id = ")
        .push_bind(id);
    let row: Option<Row> = qb.build_query_as().fetch_optional(&st.postgres).await?;
    row.map(Row::dashboard)
        .ok_or_else(|| ApiError::NotFound(format!("Dashboard {id} not found")))
}

/// Short, URL-friendly ids like `k3x-9vq-2mf`.
pub(super) fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ SEQ.fetch_add(0x9E37_79B9_7F4A_7C15, std::sync::atomic::Ordering::Relaxed);
    let mut x = n.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let alphabet = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut out = String::new();
    for i in 0..9 {
        if i > 0 && i % 3 == 0 {
            out.push('-');
        }
        out.push(alphabet[(x % 32) as usize] as char);
        x /= 32;
    }
    out
}

pub(super) fn ilike(s: &str) -> String {
    format!(
        "%{}%",
        s.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

// ================================================================ search

#[derive(Deserialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresetList {
    All,
    Custom,
    Integrations,
    Mine,
    Starred,
    Deleted,
}

#[derive(Deserialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    Title,
    Modified,
    Popularity,
}

#[derive(Deserialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Deserialize, ToSchema)]
pub struct DashboardSort {
    pub field: SortField,
    pub dir: SortDir,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSearchRequest {
    #[serde(default)]
    pub query: String,
    pub preset: PresetList,
    pub list_id: Option<String>,
    pub team: Option<String>,
    pub sort: DashboardSort,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DashboardSearchPage {
    pub dashboards: Vec<DashboardSummary>,
    pub total: i64,
}

/// Dashboards matching a preset list, custom list, team and title/author text; starred first.
#[utoipa::path(post, path = "/api/v1/dashboards/search", operation_id = "search_dashboards", request_body = DashboardSearchRequest, responses((status = 200, body = DashboardSearchPage)), tag = "dashboards")]
pub async fn search(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DashboardSearchRequest>,
) -> ApiResult<DashboardSearchPage> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(SELECT);
    qb.push(", NULL::jsonb AS definition, count(*) OVER () AS total")
        .push(FROM)
        .push_bind(&user)
        .push("::uuid WHERE d.org_id = ")
        .push_bind(&org);
    qb.push(if req.preset == PresetList::Deleted {
        " AND d.deleted_at IS NOT NULL"
    } else {
        " AND d.deleted_at IS NULL"
    });
    let hidden = hidden_logos(&*enabled_integrations(&st, &org).await?);
    qb.push(" AND NOT (d.kind = 'integration' AND d.logo = ANY(").push_bind(hidden.clone()).push("))");
    match req.preset {
        PresetList::Custom => {
            qb.push(" AND d.kind = 'custom'");
        }
        PresetList::Integrations => {
            qb.push(" AND d.kind = 'integration'");
        }
        PresetList::Mine => {
            qb.push(" AND d.author_id = ")
                .push_bind(&user)
                .push("::uuid");
        }
        PresetList::Starred => {
            qb.push(" AND s.starred");
        }
        PresetList::All | PresetList::Deleted => {}
    }
    if let Some(list) = &req.list_id {
        qb.push(
            " AND d.id = ANY(SELECT unnest(dashboard_ids) FROM dashboard_lists WHERE org_id = ",
        )
        .push_bind(&org)
        .push(" AND id = ")
        .push_bind(list)
        .push(")");
    }
    if let Some(team) = &req.team {
        qb.push(" AND ").push_bind(team).push(" = ANY(d.teams)");
    }
    let text = req.query.trim();
    if !text.is_empty() {
        let p = ilike(text);
        qb.push(" AND (d.title ILIKE ")
            .push_bind(p.clone())
            .push(" OR u.name ILIKE ")
            .push_bind(p)
            .push(")");
    }
    let dir = match req.sort.dir {
        SortDir::Asc => "ASC",
        SortDir::Desc => "DESC",
    };
    let field = match req.sort.field {
        SortField::Title => "lower(d.title)",
        SortField::Modified => "d.modified_at",
        SortField::Popularity => "d.view_count",
    };
    qb.push(format!(" ORDER BY starred DESC, {field} {dir}, d.id"));
    qb.push(" LIMIT ")
        .push_bind(req.limit.clamp(1, 200))
        .push(" OFFSET ")
        .push_bind(req.offset.max(0));
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&st.postgres).await?;
    let total = rows.first().and_then(|r| r.total).unwrap_or(0);
    Ok(Json(DashboardSearchPage {
        total,
        dashboards: rows.into_iter().map(Row::summary).collect(),
    }))
}

#[derive(Deserialize, IntoParams)]
pub struct RecentParams {
    /// Default 10, at most 50.
    pub limit: Option<i64>,
}

/// Dashboards this user opened most recently.
#[utoipa::path(get, path = "/api/v1/dashboards/recent", params(RecentParams), responses((status = 200, body = Vec<DashboardSummary>)), tag = "dashboards")]
pub async fn recent(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    CurrentUser(user): CurrentUser,
    Query(p): Query<RecentParams>,
) -> ApiResult<Vec<DashboardSummary>> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(SELECT);
    qb.push(", NULL::jsonb AS definition, NULL::bigint AS total")
        .push(FROM)
        .push_bind(&user)
        .push("::uuid WHERE d.org_id = ")
        .push_bind(&org);
    let hidden = hidden_logos(&*enabled_integrations(&st, &org).await?);
    qb.push(" AND NOT (d.kind = 'integration' AND d.logo = ANY(").push_bind(hidden).push("))");
    qb.push(
        " AND d.deleted_at IS NULL AND s.viewed_at IS NOT NULL ORDER BY s.viewed_at DESC LIMIT ",
    )
    .push_bind(p.limit.unwrap_or(10).clamp(1, 50));
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(Row::summary).collect()))
}

// ================================================================ CRUD

/// Opens a dashboard: also records the view (recently viewed, popularity).
#[utoipa::path(get, path = "/api/v1/dashboards/{id}", params(("id" = String, Path)), responses((status = 200, body = Dashboard), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn get_one(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Dashboard> {
    let updated = sqlx::query(
        "UPDATE dashboards SET view_count = view_count + 1 WHERE org_id = $1 AND id = $2",
    )
    .bind(&org)
    .bind(&id)
    .execute(&st.postgres)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Dashboard {id} not found")));
    }
    sqlx::query(
        "INSERT INTO dashboard_user_state (dashboard_id, user_id, viewed_at) VALUES ($1, $2::uuid, now()) \
         ON CONFLICT (dashboard_id, user_id) DO UPDATE SET viewed_at = now()",
    )
    .bind(&id)
    .bind(&user)
    .execute(&st.postgres)
    .await?;
    Ok(Json(load(&st, &org, &user, &id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateDashboardRequest {
    pub title: String,
    #[serde(default)]
    pub teams: Vec<String>,
    pub layout_type: LayoutType,
}

fn check_title(t: &str) -> Result<String, ApiError> {
    let t = t.trim();
    if t.is_empty() || t.chars().count() > 200 {
        return Err(ApiError::BadRequest(
            "title must be 1–200 characters".into(),
        ));
    }
    Ok(t.to_string())
}

#[utoipa::path(post, path = "/api/v1/dashboards", request_body = CreateDashboardRequest, responses((status = 200, body = Dashboard)), tag = "dashboards")]
pub async fn create(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateDashboardRequest>,
) -> ApiResult<Dashboard> {
    let title = check_title(&req.title)?;
    let id = new_id();
    let def = json!({ "layoutType": req.layout_type, "templateVariables": [], "widgets": [] });
    sqlx::query("INSERT INTO dashboards (id, org_id, title, kind, author_id, teams, definition) VALUES ($1, $2, $3, 'custom', $4::uuid, $5, $6)")
        .bind(&id)
        .bind(&org)
        .bind(&title)
        .bind(&user)
        .bind(&req.teams)
        .bind(&def)
        .execute(&st.postgres)
        .await?;
    get_one(State(st), OrgId(org), CurrentUser(user), Path(id)).await
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub widgets: Option<Vec<Value>>,
    pub template_variables: Option<Vec<TemplateVariable>>,
    pub layout_type: Option<LayoutType>,
    pub teams: Option<Vec<String>>,
    /// Per-user; doesn't change the dashboard's modified time.
    pub starred: Option<bool>,
}

#[utoipa::path(patch, path = "/api/v1/dashboards/{id}", params(("id" = String, Path)), request_body = DashboardPatch, responses((status = 200, body = Dashboard), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn update(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(p): Json<DashboardPatch>,
) -> ApiResult<Dashboard> {
    let mut tx = st.postgres.begin().await?;
    let def: Option<(Value,)> = sqlx::query_as(
        "SELECT definition FROM dashboards WHERE org_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(&org)
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((mut def,)) = def else {
        return Err(ApiError::NotFound(format!("Dashboard {id} not found")));
    };
    let content_changed = p.title.is_some()
        || p.description.is_some()
        || p.widgets.is_some()
        || p.template_variables.is_some()
        || p.layout_type.is_some()
        || p.teams.is_some();
    if content_changed {
        if let Some(w) = p.widgets {
            if w.len() > 500 {
                return Err(ApiError::BadRequest(
                    "a dashboard may hold at most 500 widgets".into(),
                ));
            }
            def["widgets"] = Value::Array(w);
        }
        if let Some(tv) = p.template_variables {
            def["templateVariables"] = json!(tv);
        }
        if let Some(l) = p.layout_type {
            def["layoutType"] = json!(l);
        }
        let title = p.title.as_deref().map(check_title).transpose()?;
        sqlx::query(
            "UPDATE dashboards SET definition = $3, title = COALESCE($4, title), description = COALESCE($5, description), \
             teams = COALESCE($6, teams), modified_at = now() WHERE org_id = $1 AND id = $2",
        )
        .bind(&org)
        .bind(&id)
        .bind(&def)
        .bind(title)
        .bind(p.description.map(|d| d.trim().to_string()))
        .bind(p.teams)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(starred) = p.starred {
        sqlx::query(
            "INSERT INTO dashboard_user_state (dashboard_id, user_id, starred) VALUES ($1, $2::uuid, $3) \
             ON CONFLICT (dashboard_id, user_id) DO UPDATE SET starred = EXCLUDED.starred",
        )
        .bind(&id)
        .bind(&user)
        .bind(starred)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(load(&st, &org, &user, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/dashboards/{id}/clone", params(("id" = String, Path)), responses((status = 200, body = Dashboard), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn clone(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Dashboard> {
    let new = new_id();
    let done = sqlx::query(
        "INSERT INTO dashboards (id, org_id, title, description, kind, logo, author_id, teams, definition) \
         SELECT $3, org_id, 'Clone of ' || title, description, 'custom', logo, $4::uuid, teams, definition FROM dashboards WHERE org_id = $1 AND id = $2",
    )
    .bind(&org)
    .bind(&id)
    .bind(&new)
    .bind(&user)
    .execute(&st.postgres)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Dashboard {id} not found")));
    }
    get_one(State(st), OrgId(org), CurrentUser(user), Path(new)).await
}

#[derive(Deserialize, ToSchema)]
pub struct IdsRequest {
    pub ids: Vec<String>,
}

/// Soft delete (restorable from the Deleted list).
#[utoipa::path(post, path = "/api/v1/dashboards/delete", request_body = IdsRequest, responses((status = 204)), tag = "dashboards")]
pub async fn delete(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    Json(req): Json<IdsRequest>,
) -> Result<StatusCode, ApiError> {
    sqlx::query("UPDATE dashboards SET deleted_at = now() WHERE org_id = $1 AND id = ANY($2) AND deleted_at IS NULL").bind(&org).bind(&req.ids).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/dashboards/{id}/restore", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn restore(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let done = sqlx::query("UPDATE dashboards SET deleted_at = NULL WHERE org_id = $1 AND id = $2")
        .bind(&org)
        .bind(&id)
        .execute(&st.postgres)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Dashboard {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ lists

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DashboardList {
    pub id: String,
    pub name: String,
    pub dashboard_ids: Vec<String>,
}

#[utoipa::path(get, path = "/api/v1/dashboard-lists", responses((status = 200, body = Vec<DashboardList>)), tag = "dashboards")]
pub async fn lists(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<DashboardList>> {
    let rows = sqlx::query_as("SELECT id, name, dashboard_ids FROM dashboard_lists WHERE org_id = $1 ORDER BY lower(name)").bind(&org).fetch_all(&st.postgres).await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateListRequest {
    pub name: String,
}

#[utoipa::path(post, path = "/api/v1/dashboard-lists", request_body = CreateListRequest, responses((status = 200, body = DashboardList)), tag = "dashboards")]
pub async fn create_list(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    Json(req): Json<CreateListRequest>,
) -> ApiResult<DashboardList> {
    let name = check_title(&req.name)?;
    let row = sqlx::query_as("INSERT INTO dashboard_lists (id, org_id, name) VALUES ($1, $2, $3) RETURNING id, name, dashboard_ids")
        .bind(format!("l-{}", new_id()))
        .bind(&org)
        .bind(&name)
        .fetch_one(&st.postgres)
        .await?;
    Ok(Json(row))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListItemsRequest {
    pub dashboard_ids: Vec<String>,
}

#[utoipa::path(post, path = "/api/v1/dashboard-lists/{id}/items", params(("id" = String, Path)), request_body = ListItemsRequest, responses((status = 200, body = DashboardList), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn add_to_list(
    State(st): State<AppState>,
    _: Writer,
    OrgId(org): OrgId,
    Path(id): Path<String>,
    Json(req): Json<ListItemsRequest>,
) -> ApiResult<DashboardList> {
    let row: Option<DashboardList> = sqlx::query_as(
        "UPDATE dashboard_lists SET dashboard_ids = ARRAY(SELECT DISTINCT unnest(dashboard_ids || $3::text[])) \
         WHERE org_id = $1 AND id = $2 RETURNING id, name, dashboard_ids",
    )
    .bind(&org)
    .bind(&id)
    .bind(&req.dashboard_ids)
    .fetch_optional(&st.postgres)
    .await?;
    row.map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("List {id} not found")))
}

// ================================================================ monitor summary widget

#[derive(Deserialize, ToSchema)]
pub struct MonitorSummaryRequest {
    /// Free text matched against monitor names.
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema, Default)]
pub struct MonitorCounts {
    pub alert: i64,
    pub warn: i64,
    pub ok: i64,
    pub nodata: i64,
}

#[derive(Serialize, ToSchema)]
pub struct MonitorRef {
    pub id: String,
    pub name: String,
    pub status: String,
    pub muted: bool,
}

#[derive(Serialize, ToSchema)]
pub struct MonitorSummary {
    pub counts: MonitorCounts,
    pub monitors: Vec<MonitorRef>,
}

#[utoipa::path(post, path = "/api/v1/monitors/summary", request_body = MonitorSummaryRequest, responses((status = 200, body = MonitorSummary)), tag = "dashboards")]
pub async fn monitor_summary(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<MonitorSummaryRequest>,
) -> ApiResult<MonitorSummary> {
    let text = req
        .query
        .split_whitespace()
        .filter(|t| !t.contains(':'))
        .collect::<Vec<_>>()
        .join(" ");
    let rows = super::monitors::summary(&st, &org, &text).await?;
    let mut counts = MonitorCounts::default();
    let monitors = rows
        .into_iter()
        .map(|(id, name, status, muted)| {
            match status {
                MonitorStatus::Alert => counts.alert += 1,
                MonitorStatus::Warn => counts.warn += 1,
                MonitorStatus::Ok => counts.ok += 1,
                MonitorStatus::NoData => counts.nodata += 1,
            }
            MonitorRef {
                id: id.to_string(),
                name,
                status: status.as_str().to_string(),
                muted,
            }
        })
        .collect();
    Ok(Json(MonitorSummary { counts, monitors }))
}

// ================================================================ seeds

/// Built-in dashboards, (re)installed for the org at startup. Integration dashboards are owned by
/// the product, so their definition is refreshed; the example custom dashboard is only created once.
const SEEDS: &[(&str, &str)] = &[
    (
        "host-overview",
        include_str!("../../../../seeds/dashboards/host-overview.json"),
    ),
    (
        "apm-services",
        include_str!("../../../../seeds/dashboards/apm-services.json"),
    ),
    (
        "service-overview-prod",
        include_str!("../../../../seeds/dashboards/service-overview-prod.json"),
    ),
    (
        "synthetics-api-tests",
        include_str!("../../../../seeds/dashboards/synthetics-api-tests.json"),
    ),
];

// Kubernetes: generated from the pinned integrations-core revision recorded on each document
// (copyright and BSD-3-Clause terms: server/seeds/integrations/kubernetes/LICENSE). AWS, Azure and
// Google Cloud: ObserveCraft's own overviews (scripts/cloud-dashboards.mjs).
const INTEGRATION_BUNDLES: &[&str] = &[
    include_str!("../../../../seeds/integrations/kubernetes/dashboards.json"),
    include_str!("../../../../seeds/integrations/aws/dashboards.json"),
    include_str!("../../../../seeds/integrations/azure/dashboards.json"),
    include_str!("../../../../seeds/integrations/gcp/dashboards.json"),
];

fn seed_documents() -> anyhow::Result<Vec<(String, Value)>> {
    let mut documents = Vec::new();
    for (id, raw) in SEEDS {
        documents.push((id.to_string(), serde_json::from_str(raw)?));
    }
    for raw in INTEGRATION_BUNDLES {
        for doc in serde_json::from_str::<Vec<Value>>(raw)? {
            let id = doc["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("integration dashboard has no id"))?
                .to_string();
            documents.push((id, doc));
        }
    }
    Ok(documents)
}

// ================================================================ available integration dashboards

/// Integration dashboards appear once their integration is enabled (by the dashboard's `logo`): a cloud
/// account, synthetic tests, or its data received in the last day. Others are always available.
const GATED: [&str; 7] = ["aws", "azure", "gcp", "kubernetes", "linux", "opentelemetry", "tile:synthetics"];

/// The gated integrations not enabled: their dashboards are left out of searches and recents.
pub fn hidden_logos(enabled: &std::collections::BTreeSet<String>) -> Vec<String> {
    GATED.iter().filter(|l| !enabled.contains(**l)).map(|l| l.to_string()).collect()
}

/// How long an org's enabled integrations are reused (an account added shows its dashboards within it).
const ENABLED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn enabled_cache() -> &'static crate::ttl_cache::TtlCache<std::collections::BTreeSet<String>> {
    static CACHE: std::sync::OnceLock<crate::ttl_cache::TtlCache<std::collections::BTreeSet<String>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| crate::ttl_cache::TtlCache::new(ENABLED_TTL).with_max_entries(10_000))
}

/// Drops the org's cached enabled integrations (tests; after configuration changes).
pub fn forget_enabled(org: &str) {
    enabled_cache().remove_prefix(&format!("{org}\0"));
}

/// Held by changes that enable or disable an integration (a cloud account added or removed): when
/// it goes out of scope, after the change is written, the org's dashboards are recomputed.
pub struct EnabledChange<'a>(pub &'a str);

impl Drop for EnabledChange<'_> {
    fn drop(&mut self) {
        forget_enabled(self.0);
    }
}

/// The org's enabled gated integrations, reused for [`ENABLED_TTL`].
pub async fn enabled_integrations(st: &AppState, org: &str) -> Result<std::sync::Arc<std::collections::BTreeSet<String>>, ApiError> {
    enabled_cache()
        .get_or_load(&format!("{org}\0"), || async {
            let (aws, azure, gcp, synthetics): (bool, bool, bool, bool) = sqlx::query_as(
                "SELECT EXISTS (SELECT 1 FROM aws_accounts WHERE org_id = $1), EXISTS (SELECT 1 FROM azure_accounts WHERE org_id = $1), \
                 EXISTS (SELECT 1 FROM gcp_accounts WHERE org_id = $1), EXISTS (SELECT 1 FROM synthetic_tests WHERE org_id = $1)",
            )
            .bind(org)
            .fetch_one(&st.postgres)
            .await?;
            #[derive(clickhouse::Row, Deserialize)]
            struct Seen {
                kubernetes: bool,
                hosts: bool,
                traces: bool,
            }
            // Data of the last day: Kubernetes objects or metrics, host metrics, spans. Scalar subqueries are
            // Nullable: ifNull keeps the columns plain UInt8.
            let seen = st
                .ch(org)
                .query(
                    "SELECT \
                       ifNull((SELECT count() FROM (SELECT 1 FROM otel.k8s_inventory WHERE OrgId = {oc_org:String} AND LastSeen > now64(3) - INTERVAL 1 DAY LIMIT 1)) \
                       + (SELECT count() FROM (SELECT 1 FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(now() - INTERVAL 1 DAY) \
                          AND (startsWith(MetricName, 'kubernetes') OR startsWith(MetricName, 'k8s.')) LIMIT 1)) > 0, 0) AS kubernetes, \
                       ifNull((SELECT count() FROM (SELECT 1 FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(now() - INTERVAL 1 DAY) \
                          AND startsWith(MetricName, 'system.') LIMIT 1)) > 0, 0) AS hosts, \
                       ifNull((SELECT count() FROM (SELECT 1 FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp > now() - INTERVAL 1 DAY LIMIT 1)) > 0, 0) AS traces",
                )
                .fetch_one::<Seen>()
                .await?;
            let flags = [("aws", aws), ("azure", azure), ("gcp", gcp), ("tile:synthetics", synthetics), ("kubernetes", seen.kubernetes), ("linux", seen.hosts), ("opentelemetry", seen.traces)];
            let mut enabled: std::collections::BTreeSet<String> = flags.into_iter().filter(|(_, on)| *on).map(|(l, _)| l.to_string()).collect();
            // integrations-core integrations sending metrics (their name's first two segments): their dashboards are
            // installed for the org the first time.
            let prefixes: std::collections::BTreeSet<String> = st
                .ch(org)
                .query(
                    "SELECT DISTINCT arrayStringConcat(arraySlice(splitByChar('.', MetricName), 1, 2), '.') FROM otel.metrics_series \
                     WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(now() - INTERVAL 1 DAY)",
                )
                .fetch_all::<String>()
                .await?
                .into_iter()
                .collect();
            let core = super::integration_dashboards::enabled_logos(&prefixes);
            if let Err(e) = super::integration_dashboards::install_missing(&st.postgres, org, &core).await {
                tracing::warn!(org, error = %e, "installing integration dashboards failed");
            }
            enabled.extend(core.into_iter().map(str::to_string));
            Ok::<_, ApiError>(enabled)
        })
        .await
}

pub async fn install_seeds(pg: &sqlx::PgPool, org: &str, user: &str) -> anyhow::Result<()> {
    for (id, doc) in seed_documents()? {
        upsert_seed(pg, org, Some(user), &id, &doc).await?;
    }
    super::integration_dashboards::refresh_installed(pg, org).await
}

/// Writes one seed dashboard for the org: integration dashboards are refreshed, others only created once.
pub(crate) async fn upsert_seed(pg: &sqlx::PgPool, org: &str, user: Option<&str>, id: &str, doc: &Value) -> anyhow::Result<()> {
    let kind = doc["kind"].as_str().unwrap_or("custom");
    let teams: Vec<String> = serde_json::from_value(doc["teams"].clone()).unwrap_or_default();
    let def = json!({ "layoutType": doc["layoutType"], "templateVariables": doc["templateVariables"], "widgets": doc["widgets"] });
    let author = if kind == "custom" { user } else { None };
    let refresh = if kind == "integration" {
        "DO UPDATE SET definition = EXCLUDED.definition, title = EXCLUDED.title, description = EXCLUDED.description WHERE dashboards.org_id = EXCLUDED.org_id"
    } else {
        "DO NOTHING"
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO dashboards (id, org_id, title, description, kind, logo, author_id, teams, definition) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $8, $9) ON CONFLICT (id) {refresh}"
    )))
    .bind(if org == "default" { id.to_string() } else { format!("{org}-{id}") })
    .bind(org)
    .bind(doc["title"].as_str().unwrap_or(id))
    .bind(doc["description"].as_str())
    .bind(kind)
    .bind(doc["logo"].as_str())
    .bind(author)
    .bind(&teams)
    .bind(&def)
    .execute(pg)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn dashboards_of_integrations_not_enabled_are_hidden() {
        let enabled: BTreeSet<String> = ["aws".to_string(), "kubernetes".to_string()].into();
        let hidden = hidden_logos(&enabled);
        assert!(hidden.contains(&"azure".to_string()) && hidden.contains(&"gcp".to_string()) && hidden.contains(&"linux".to_string()));
        assert!(!hidden.contains(&"aws".to_string()) && !hidden.contains(&"kubernetes".to_string()));
        assert_eq!(hidden_logos(&GATED.iter().map(|s| s.to_string()).collect()), Vec::<String>::new());
    }

    /// A new org sees none of the gated integration dashboards; an AWS account brings AWS Overview,
    /// Kubernetes objects the Kubernetes ones, and other orgs' data changes nothing.
    #[tokio::test]
    async fn integration_dashboards_appear_once_their_integration_is_enabled() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "dash-gating").await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Dashboards Tester') RETURNING id::text")
            .bind(&org)
            .bind(format!("{org}@example.com"))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        install_seeds(&st.postgres, &org, &user).await.unwrap();
        let ids = |st: AppState, org: String, user: String| async move {
            let req = DashboardSearchRequest { query: String::new(), preset: PresetList::Integrations, list_id: None, team: None, sort: DashboardSort { field: SortField::Title, dir: SortDir::Asc }, offset: 0, limit: 200 };
            let Json(page) = search(State(st), OrgId(org.clone()), CurrentUser(user), Json(req)).await.unwrap();
            page.dashboards.into_iter().map(|d| d.id.trim_start_matches(&format!("{org}-")).to_string()).collect::<Vec<_>>()
        };
        let before = ids(st.clone(), org.clone(), user.clone()).await;
        for id in ["aws-overview", "azure-overview", "gcp-overview", "kubernetes-kubernetes-dashboard", "host-overview", "apm-services", "synthetics-api-tests"] {
            assert!(!before.contains(&id.to_string()), "{id} is hidden: {before:?}");
        }

        // Adding an account shows its dashboards at once (no wait for the cache).
        let account = serde_json::json!({ "aws_account_id": "123456789012", "auth_config": { "role_name": "ObserveCraftIntegrationRole" } });
        crate::routes::datadog_api::aws_integration::create_row(st, &org, account.as_object().unwrap()).await.unwrap();
        let with_aws = ids(st.clone(), org.clone(), user.clone()).await;
        assert!(with_aws.contains(&"aws-overview".to_string()), "{with_aws:?}");
        st.ch(&org)
            .query("INSERT INTO otel.otel_k8s_objects (Timestamp, Body, ResourceAttributes) VALUES (now64(9), ?, map('oc.org_id', ?, 'k8s.cluster.name', 'shop-prod', 'oc.signal', 'k8s.objects'))")
            .bind(serde_json::json!({ "kind": "Namespace", "metadata": { "uid": format!("{org}-ns"), "name": "checkout" } }).to_string())
            .bind(&org)
            .execute()
            .await
            .unwrap();
        forget_enabled(&org);
        let after = ids(st.clone(), org.clone(), user.clone()).await;
        assert!(after.contains(&"aws-overview".to_string()) && after.contains(&"kubernetes-kubernetes-dashboard".to_string()), "{after:?}");
        assert!(!after.contains(&"azure-overview".to_string()) && !after.contains(&"gcp-overview".to_string()), "{after:?}");

        let other = crate::test_support::new_org(st, "dash-gating-other").await;
        assert!(enabled_integrations(st, &other).await.unwrap().is_empty());
    }
}
