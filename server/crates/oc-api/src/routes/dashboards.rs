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

// Generated from the pinned integrations-core revision recorded on each document.
// Copyright and BSD-3-Clause terms: server/seeds/integrations/kubernetes/LICENSE.
const INTEGRATION_BUNDLES: &[&str] = &[include_str!(
    "../../../../seeds/integrations/kubernetes/dashboards.json"
)];

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

pub async fn install_seeds(pg: &sqlx::PgPool, org: &str, user: &str) -> anyhow::Result<()> {
    for (id, doc) in seed_documents()? {
        let kind = doc["kind"].as_str().unwrap_or("custom");
        let teams: Vec<String> = serde_json::from_value(doc["teams"].clone()).unwrap_or_default();
        let def = json!({ "layoutType": doc["layoutType"], "templateVariables": doc["templateVariables"], "widgets": doc["widgets"] });
        let author = (kind == "custom").then_some(user);
        let refresh = if kind == "integration" {
            "DO UPDATE SET definition = EXCLUDED.definition, title = EXCLUDED.title, description = EXCLUDED.description WHERE dashboards.org_id = EXCLUDED.org_id"
        } else {
            "DO NOTHING"
        };
        sqlx::query(&format!(
            "INSERT INTO dashboards (id, org_id, title, description, kind, logo, author_id, teams, definition) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $8, $9) ON CONFLICT (id) {refresh}"
        ))
        .bind(if org == "default" { id.to_string() } else { format!("{org}-{id}") })
        .bind(org)
        .bind(doc["title"].as_str().unwrap_or(&id))
        .bind(doc["description"].as_str())
        .bind(kind)
        .bind(doc["logo"].as_str())
        .bind(author)
        .bind(&teams)
        .bind(&def)
        .execute(pg)
        .await?;
    }
    Ok(())
}
