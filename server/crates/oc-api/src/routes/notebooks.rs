//! Notebooks (Dashboards → Notebooks): documents of ordered cells — Markdown text and live query
//! cells (timeseries, query value, top list, log stream) — with a time range of their own.
//!
//! A row holds the metadata in columns and the cells as the JSON the web app edits
//! (`web/src/api/notebooks/types.ts`): `{ id, definition, time? }`, where `definition` has the shape
//! of a dashboard widget definition (or `{ type: "markdown", text }`). The server checks only the
//! cell types and sizes (`validate_cells`). Per-user state lives in `notebook_favorites` and
//! `notebook_views` (Recents). Deleting is soft until the notebook is restored. Every query is
//! scoped by org.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use utoipa::ToSchema;

use super::dashboards::{ilike, new_id, SortDir};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub(crate) const MAX_CELLS: usize = 200;
pub(crate) const MAX_MARKDOWN_CHARS: usize = 50_000;
pub(crate) const MAX_REQUESTS: usize = 4;
const MAX_QUERY_CHARS: usize = 2_000;
const MAX_CELLS_BYTES: usize = 1_000_000;
const MAX_LABELS: usize = 20;
const MAX_LABEL_CHARS: usize = 100;
/// The relative ranges of the time picker (web/src/components/shell/TimeContext.tsx).
const TIME_RANGES: &[&str] = &["15m", "1h", "4h", "1d", "2d", "1w"];
const CELL_TYPES: &str = "markdown, timeseries, query_value, toplist, log_stream";

// ================================================================ validation

/// Checks the cells of a notebook: known types, the fields each type needs, and size limits.
pub(crate) fn validate_cells(cells: &[Value]) -> Result<(), String> {
    if cells.len() > MAX_CELLS {
        return Err(format!("a notebook holds at most {MAX_CELLS} cells"));
    }
    if serde_json::to_string(cells).map_or(0, |s| s.len()) > MAX_CELLS_BYTES {
        return Err(format!("the cells of a notebook take at most {} KB", MAX_CELLS_BYTES / 1000));
    }
    let mut ids = HashSet::new();
    for (i, cell) in cells.iter().enumerate() {
        validate_cell(cell).map_err(|e| format!("cell {}: {e}", i + 1))?;
        let id = cell["id"].as_str().unwrap_or_default();
        if !ids.insert(id) {
            return Err(format!("cell {}: the id '{id}' is used twice", i + 1));
        }
    }
    Ok(())
}

fn validate_cell(cell: &Value) -> Result<(), String> {
    let Some(obj) = cell.as_object() else {
        return Err("a cell is an object with an id and a definition".into());
    };
    let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
    if id.is_empty() || id.chars().count() > 64 {
        return Err("a cell needs an id of 1–64 characters".into());
    }
    match obj.get("time") {
        None | Some(Value::Null) => {}
        Some(t) => check_time(t.as_str().unwrap_or_default())?,
    }
    let def = &cell["definition"];
    match def["type"].as_str().unwrap_or_default() {
        "markdown" => {
            let Some(text) = def["text"].as_str() else {
                return Err("a Markdown cell needs its text".into());
            };
            if text.chars().count() > MAX_MARKDOWN_CHARS {
                return Err(format!("a Markdown cell is at most {MAX_MARKDOWN_CHARS} characters"));
            }
        }
        "timeseries" => {
            let requests = def["requests"].as_array().map(Vec::as_slice).unwrap_or_default();
            if requests.is_empty() {
                return Err("a timeseries cell needs a metric query".into());
            }
            if requests.len() > MAX_REQUESTS {
                return Err(format!("a timeseries cell graphs at most {MAX_REQUESTS} queries"));
            }
            for r in requests {
                check_metric_query(&r["query"])?;
            }
        }
        "query_value" | "toplist" => check_metric_query(&def["request"]["query"])?,
        "log_stream" => {
            check_search(&def["query"])?;
            if !(1..=100).contains(&def["limit"].as_u64().unwrap_or(0)) {
                return Err("a log stream shows a limit of 1–100 logs".into());
            }
        }
        other => return Err(format!("unsupported cell type '{other}' (expected one of {CELL_TYPES})")),
    }
    Ok(())
}

fn check_metric_query(q: &Value) -> Result<(), String> {
    if q.as_str().unwrap_or_default().trim().is_empty() {
        return Err("a query cell needs a metric query".into());
    }
    check_search(q)
}

/// A query string, possibly empty (a log stream of every log).
fn check_search(q: &Value) -> Result<(), String> {
    match q {
        Value::String(s) if s.chars().count() <= MAX_QUERY_CHARS => Ok(()),
        Value::String(_) => Err(format!("a query is at most {MAX_QUERY_CHARS} characters")),
        _ => Err("the query must be text".into()),
    }
}

fn check_time(t: &str) -> Result<(), String> {
    if TIME_RANGES.contains(&t) {
        Ok(())
    } else {
        Err(format!("unknown time range '{t}' (expected one of {})", TIME_RANGES.join(", ")))
    }
}

fn check_title(t: &str) -> Result<String, String> {
    let t = t.trim();
    if t.is_empty() || t.chars().count() > 200 {
        return Err("the title must be 1–200 characters".into());
    }
    Ok(t.to_string())
}

/// Teams or tags: trimmed, without blanks and duplicates, at most `MAX_LABELS` of `MAX_LABEL_CHARS`.
fn clean_labels(list: Vec<String>, what: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for label in list {
        let label = label.trim().to_string();
        if label.is_empty() || out.contains(&label) {
            continue;
        }
        if label.chars().count() > MAX_LABEL_CHARS {
            return Err(format!("{what} are at most {MAX_LABEL_CHARS} characters each"));
        }
        out.push(label);
    }
    if out.len() > MAX_LABELS {
        return Err(format!("a notebook has at most {MAX_LABELS} {what}"));
    }
    Ok(out)
}

fn bad(e: String) -> ApiError {
    ApiError::BadRequest(e)
}

fn not_found(id: &str) -> ApiError {
    ApiError::NotFound(format!("Notebook {id} not found"))
}

// ================================================================ model

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum NotebookStatus {
    Published,
    Draft,
}

impl NotebookStatus {
    fn as_str(self) -> &'static str {
        match self {
            NotebookStatus::Published => "published",
            NotebookStatus::Draft => "draft",
        }
    }
}

#[derive(Serialize, ToSchema, Clone, sqlx::FromRow)]
pub struct NotebookAuthor {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotebookSummary {
    pub id: String,
    pub title: String,
    pub author: Option<NotebookAuthor>,
    pub teams: Vec<String>,
    pub tags: Vec<String>,
    pub status: NotebookStatus,
    /// The notebook's relative time range (`1h`); cells may override it.
    pub time: String,
    pub cell_count: i64,
    pub created_at: i64,
    pub modified_at: i64,
    /// When the current user last opened it.
    pub viewed_at: Option<i64>,
    /// The current user's favorite.
    pub favorite: bool,
    pub deleted_at: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct Notebook {
    #[serde(flatten)]
    pub summary: NotebookSummary,
    /// `{ id, definition, time? }` documents (schema owned by the web app).
    #[schema(value_type = Vec<Object>)]
    pub cells: Vec<Value>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    title: String,
    author_id: Option<String>,
    author_name: Option<String>,
    teams: Vec<String>,
    tags: Vec<String>,
    status: String,
    time_range: String,
    cell_count: i64,
    created_at: i64,
    modified_at: i64,
    viewed_at: Option<i64>,
    favorite: bool,
    deleted_at: Option<i64>,
    cells: Option<Value>,
    total: i64,
}

impl Row {
    fn split(self) -> (NotebookSummary, Option<Value>) {
        let summary = NotebookSummary {
            author: self.author_id.map(|id| NotebookAuthor { id, name: self.author_name.unwrap_or_default() }),
            status: if self.status == "draft" { NotebookStatus::Draft } else { NotebookStatus::Published },
            id: self.id,
            title: self.title,
            teams: self.teams,
            tags: self.tags,
            time: self.time_range,
            cell_count: self.cell_count,
            created_at: self.created_at,
            modified_at: self.modified_at,
            viewed_at: self.viewed_at,
            favorite: self.favorite,
            deleted_at: self.deleted_at,
        };
        (summary, self.cells)
    }

    fn summary(self) -> NotebookSummary {
        self.split().0
    }

    fn notebook(self) -> Notebook {
        let (summary, cells) = self.split();
        let cells = match cells {
            Some(Value::Array(cells)) => cells,
            _ => Vec::new(),
        };
        Notebook { summary, cells }
    }
}

/// Notebooks of `org` with the per-user state of `user`; callers append ` AND …` conditions.
fn base_query<'a>(org: &'a str, user: &'a str, with_cells: bool) -> QueryBuilder<'a, Postgres> {
    let mut qb = QueryBuilder::new(
        "SELECT n.id, n.title, u.id::text AS author_id, u.name AS author_name, n.teams, n.tags, n.status, n.time_range, \
         jsonb_array_length(n.cells)::bigint AS cell_count, \
         (extract(epoch FROM n.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM n.modified_at) * 1000)::bigint AS modified_at, \
         (extract(epoch FROM v.viewed_at) * 1000)::bigint AS viewed_at, (f.user_id IS NOT NULL) AS favorite, \
         (extract(epoch FROM n.deleted_at) * 1000)::bigint AS deleted_at, ",
    );
    qb.push(if with_cells { "n.cells" } else { "NULL::jsonb AS cells" });
    qb.push(
        ", count(*) OVER () AS total FROM notebooks n LEFT JOIN users u ON u.id = n.author_id \
         LEFT JOIN notebook_views v ON v.org_id = n.org_id AND v.notebook_id = n.id AND v.user_id = ",
    )
    .push_bind(user)
    .push("::uuid LEFT JOIN notebook_favorites f ON f.org_id = n.org_id AND f.notebook_id = n.id AND f.user_id = ")
    .push_bind(user)
    .push("::uuid WHERE n.org_id = ")
    .push_bind(org);
    qb
}

async fn load(st: &AppState, org: &str, user: &str, id: &str) -> Result<Notebook, ApiError> {
    let mut qb = base_query(org, user, true);
    qb.push(" AND n.id = ").push_bind(id);
    let row: Option<Row> = qb.build_query_as().fetch_optional(&st.postgres).await?;
    row.map(Row::notebook).ok_or_else(|| not_found(id))
}

// ================================================================ search

/// The lists of the notebook list's left navigation.
#[derive(Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum NotebookListView {
    All,
    /// Written by the current user.
    Mine,
    /// Tagged with a team the current user belongs to.
    Team,
    /// Opened by the current user, the latest first.
    Recents,
    Favorites,
    /// Soft-deleted, restorable.
    Deleted,
}

#[derive(Deserialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum NotebookSortField {
    Title,
    Modified,
    Created,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotebookSearchRequest {
    /// Matched against titles, authors, tags and the text of the cells.
    #[serde(default)]
    pub query: String,
    pub view: NotebookListView,
    /// An author's user id.
    pub author: Option<String>,
    /// A team handle.
    pub team: Option<String>,
    /// Ignored by Recents, which is ordered by the last visit.
    pub sort: NotebookSortField,
    pub dir: SortDir,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
pub struct NotebookSearchPage {
    pub notebooks: Vec<NotebookSummary>,
    pub total: i64,
    /// Everyone who wrote one of the org's notebooks (the author filter).
    pub authors: Vec<NotebookAuthor>,
    /// The teams the org's notebooks are tagged with (the team filter).
    pub teams: Vec<String>,
}

/// The search behind the notebook list, scoped to `org` and the per-user lists of `user`.
fn search_query<'a>(org: &'a str, user: &'a str, req: &'a NotebookSearchRequest) -> QueryBuilder<'a, Postgres> {
    let mut qb = base_query(org, user, false);
    qb.push(if req.view == NotebookListView::Deleted { " AND n.deleted_at IS NOT NULL" } else { " AND n.deleted_at IS NULL" });
    match req.view {
        NotebookListView::Mine => {
            qb.push(" AND n.author_id = ").push_bind(user).push("::uuid");
        }
        NotebookListView::Team => {
            qb.push(" AND n.teams && ARRAY(SELECT tm.handle FROM team_members tm WHERE tm.org_id = n.org_id AND tm.user_id = ").push_bind(user).push("::uuid)");
        }
        NotebookListView::Recents => {
            qb.push(" AND v.viewed_at IS NOT NULL");
        }
        NotebookListView::Favorites => {
            qb.push(" AND f.user_id IS NOT NULL");
        }
        NotebookListView::All | NotebookListView::Deleted => {}
    }
    if let Some(author) = req.author.as_deref().filter(|a| !a.is_empty()) {
        qb.push(" AND n.author_id::text = ").push_bind(author);
    }
    if let Some(team) = req.team.as_deref().filter(|t| !t.is_empty()) {
        qb.push(" AND ").push_bind(team).push(" = ANY(n.teams)");
    }
    let text = req.query.trim();
    if !text.is_empty() {
        let p = ilike(text);
        qb.push(" AND (n.title ILIKE ")
            .push_bind(p.clone())
            .push(" OR u.name ILIKE ")
            .push_bind(p.clone())
            .push(" OR array_to_string(n.tags, ' ') ILIKE ")
            .push_bind(p.clone())
            .push(" OR n.cells::text ILIKE ")
            .push_bind(p)
            .push(")");
    }
    if req.view == NotebookListView::Recents {
        qb.push(" ORDER BY v.viewed_at DESC, n.id");
    } else {
        let field = match req.sort {
            NotebookSortField::Title => "lower(n.title)",
            NotebookSortField::Modified => "n.modified_at",
            NotebookSortField::Created => "n.created_at",
        };
        let dir = match req.dir {
            SortDir::Asc => "ASC",
            SortDir::Desc => "DESC",
        };
        qb.push(format!(" ORDER BY {field} {dir}, n.id"));
    }
    qb.push(" LIMIT ").push_bind(req.limit.clamp(1, 200)).push(" OFFSET ").push_bind(req.offset.max(0));
    qb
}

/// Notebooks of a list (mine, team, recents, favorites, all, recently deleted), filtered by text, author and team.
#[utoipa::path(post, path = "/api/v1/notebooks/search", operation_id = "search_notebooks", request_body = NotebookSearchRequest, responses((status = 200, body = NotebookSearchPage)), tag = "notebooks")]
pub async fn search(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<NotebookSearchRequest>) -> ApiResult<NotebookSearchPage> {
    let rows: Vec<Row> = search_query(&ctx.org_id, &ctx.user_id, &req).build_query_as().fetch_all(&st.postgres).await?;
    let authors: Vec<NotebookAuthor> =
        sqlx::query_as("SELECT DISTINCT u.id::text AS id, u.name FROM notebooks n JOIN users u ON u.id = n.author_id WHERE n.org_id = $1 AND n.deleted_at IS NULL ORDER BY u.name")
            .bind(&ctx.org_id)
            .fetch_all(&st.postgres)
            .await?;
    let teams: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT t FROM notebooks n, unnest(n.teams) t WHERE n.org_id = $1 AND n.deleted_at IS NULL ORDER BY t").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(NotebookSearchPage { total: rows.first().map_or(0, |r| r.total), notebooks: rows.into_iter().map(Row::summary).collect(), authors, teams: teams.into_iter().map(|(t,)| t).collect() }))
}

// ================================================================ CRUD

/// Opens a notebook and records the visit (Recents). A deleted notebook still opens, to be restored.
#[utoipa::path(get, path = "/api/v1/notebooks/{id}", operation_id = "get_notebook", params(("id" = String, Path)), responses((status = 200, body = Notebook), (status = 404, body = crate::error::ErrorBody)), tag = "notebooks")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Notebook> {
    sqlx::query(
        "INSERT INTO notebook_views (org_id, notebook_id, user_id) SELECT org_id, id, $3::uuid FROM notebooks WHERE org_id = $1 AND id = $2 \
         ON CONFLICT (notebook_id, user_id) DO UPDATE SET viewed_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateNotebookRequest {
    pub title: String,
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub status: Option<NotebookStatus>,
    /// Relative time range, `1h` by default.
    pub time: Option<String>,
    /// The first cells (e.g. from a template).
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub cells: Vec<Value>,
}

#[utoipa::path(post, path = "/api/v1/notebooks", operation_id = "create_notebook", request_body = CreateNotebookRequest, responses((status = 200, body = Notebook)), tag = "notebooks")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<CreateNotebookRequest>) -> ApiResult<Notebook> {
    let title = check_title(&req.title).map_err(bad)?;
    let teams = clean_labels(req.teams, "teams").map_err(bad)?;
    let tags = clean_labels(req.tags, "tags").map_err(bad)?;
    let time = req.time.unwrap_or_else(|| "1h".into());
    check_time(&time).map_err(bad)?;
    validate_cells(&req.cells).map_err(bad)?;
    let id = new_id();
    sqlx::query("INSERT INTO notebooks (id, org_id, title, author_id, teams, tags, status, time_range, cells) VALUES ($1, $2, $3, $4::uuid, $5, $6, $7, $8, $9)")
        .bind(&id)
        .bind(&ctx.org_id)
        .bind(&title)
        .bind(&ctx.user_id)
        .bind(&teams)
        .bind(&tags)
        .bind(req.status.unwrap_or(NotebookStatus::Published).as_str())
        .bind(&time)
        .bind(Value::Array(req.cells))
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "notebook.created", &id, &title).await;
    get_one(State(st), ctx, Path(id)).await
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotebookPatch {
    pub title: Option<String>,
    pub teams: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub status: Option<NotebookStatus>,
    pub time: Option<String>,
    /// Replaces every cell (order included).
    #[schema(value_type = Option<Vec<Object>>)]
    pub cells: Option<Vec<Value>>,
}

impl NotebookPatch {
    /// The changed fields, for the audit trail.
    fn changed(&self) -> String {
        [
            ("title", self.title.is_some()),
            ("teams", self.teams.is_some()),
            ("tags", self.tags.is_some()),
            ("status", self.status.is_some()),
            ("time", self.time.is_some()),
            ("cells", self.cells.is_some()),
        ]
        .iter()
        .filter(|(_, set)| *set)
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// Saves changes (the editor autosaves). A deleted notebook must be restored first.
#[utoipa::path(patch, path = "/api/v1/notebooks/{id}", operation_id = "update_notebook", params(("id" = String, Path)), request_body = NotebookPatch, responses((status = 200, body = Notebook), (status = 404, body = crate::error::ErrorBody)), tag = "notebooks")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(p): Json<NotebookPatch>) -> ApiResult<Notebook> {
    let changed = p.changed();
    if changed.is_empty() {
        return Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?));
    }
    let title = p.title.as_deref().map(check_title).transpose().map_err(bad)?;
    let teams = p.teams.map(|t| clean_labels(t, "teams")).transpose().map_err(bad)?;
    let tags = p.tags.map(|t| clean_labels(t, "tags")).transpose().map_err(bad)?;
    if let Some(t) = &p.time {
        check_time(t).map_err(bad)?;
    }
    if let Some(cells) = &p.cells {
        validate_cells(cells).map_err(bad)?;
    }
    let done = sqlx::query(
        "UPDATE notebooks SET title = COALESCE($3, title), teams = COALESCE($4, teams), tags = COALESCE($5, tags), status = COALESCE($6, status), \
         time_range = COALESCE($7, time_range), cells = COALESCE($8, cells), modified_at = now() WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(title)
    .bind(teams)
    .bind(tags)
    .bind(p.status.map(NotebookStatus::as_str))
    .bind(p.time)
    .bind(p.cells.map(Value::Array))
    .execute(&st.postgres)
    .await?;
    if done.rows_affected() == 0 {
        let deleted = load(&st, &ctx.org_id, &ctx.user_id, &id).await?.summary.deleted_at.is_some();
        return Err(if deleted { ApiError::BadRequest("This notebook is deleted: restore it to edit it".into()) } else { not_found(&id) });
    }
    ctx.audit(&st, "notebook.updated", &id, &changed).await;
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

/// A copy owned by the current user, titled "Clone of …".
#[utoipa::path(post, path = "/api/v1/notebooks/{id}/clone", operation_id = "clone_notebook", params(("id" = String, Path)), responses((status = 200, body = Notebook), (status = 404, body = crate::error::ErrorBody)), tag = "notebooks")]
pub async fn clone(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<Notebook> {
    let copy = new_id();
    let done = sqlx::query(
        "INSERT INTO notebooks (id, org_id, title, author_id, teams, tags, status, time_range, cells) \
         SELECT $3, org_id, left('Clone of ' || title, 200), $4::uuid, teams, tags, status, time_range, cells FROM notebooks WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&copy)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    if done.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    ctx.audit(&st, "notebook.cloned", &copy, &format!("from {id}")).await;
    get_one(State(st), ctx, Path(copy)).await
}

const SOFT_DELETE: &str = "UPDATE notebooks SET deleted_at = now() WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL";
const RESTORE: &str = "UPDATE notebooks SET deleted_at = NULL WHERE org_id = $1 AND id = $2 AND deleted_at IS NOT NULL";

/// Moves a notebook to Recently Deleted (restorable).
#[utoipa::path(delete, path = "/api/v1/notebooks/{id}", operation_id = "delete_notebook", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "notebooks")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let done = sqlx::query(SOFT_DELETE).bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    ctx.audit(&st, "notebook.deleted", &id, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/notebooks/{id}/restore", operation_id = "restore_notebook", params(("id" = String, Path)), responses((status = 200, body = Notebook), (status = 404, body = crate::error::ErrorBody)), tag = "notebooks")]
pub async fn restore(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<Notebook> {
    let done = sqlx::query(RESTORE).bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Notebook {id} is not in Recently Deleted")));
    }
    ctx.audit(&st, "notebook.restored", &id, "").await;
    Ok(Json(load(&st, &ctx.org_id, &ctx.user_id, &id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct NotebookFavoriteRequest {
    pub favorite: bool,
}

/// Stars or unstars a notebook for the current user (any role: it changes only their own list).
#[utoipa::path(put, path = "/api/v1/notebooks/{id}/favorite", operation_id = "set_notebook_favorite", params(("id" = String, Path)), request_body = NotebookFavoriteRequest, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "notebooks")]
pub async fn set_favorite(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(req): Json<NotebookFavoriteRequest>) -> Result<StatusCode, ApiError> {
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM notebooks WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    if exists.is_none() {
        return Err(not_found(&id));
    }
    let sql = if req.favorite {
        "INSERT INTO notebook_favorites (org_id, notebook_id, user_id) VALUES ($1, $2, $3::uuid) ON CONFLICT (notebook_id, user_id) DO NOTHING"
    } else {
        "DELETE FROM notebook_favorites WHERE org_id = $1 AND notebook_id = $2 AND user_id = $3::uuid"
    };
    sqlx::query(sql).bind(&ctx.org_id).bind(&id).bind(&ctx.user_id).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn markdown(id: &str, text: &str) -> Value {
        json!({ "id": id, "definition": { "type": "markdown", "text": text } })
    }

    #[test]
    fn accepts_the_supported_cell_types() {
        let cells = vec![
            markdown("a", "# Checkout latency"),
            json!({ "id": "b", "definition": { "type": "timeseries", "display": "line", "requests": [{ "query": "avg:trace.http.request.duration{service:order-service}" }] }, "time": "4h" }),
            json!({ "id": "c", "definition": { "type": "query_value", "request": { "query": "sum:trace.http.request.errors{service:billing}" }, "aggregator": "sum", "precision": 0, "formats": [] } }),
            json!({ "id": "d", "definition": { "type": "toplist", "request": { "query": "avg:system.cpu.user{*} by {host}" }, "aggregator": "avg", "limit": 10 } }),
            json!({ "id": "e", "definition": { "type": "log_stream", "query": "service:order-service status:error", "limit": 50 } }),
        ];
        assert_eq!(validate_cells(&cells), Ok(()));
    }

    #[test]
    fn rejects_unknown_or_malformed_cells() {
        let err = |cells: Vec<Value>| validate_cells(&cells).unwrap_err();
        assert!(err(vec![json!({ "id": "a", "definition": { "type": "iframe" } })]).contains("unsupported"));
        assert!(err(vec![json!({ "definition": { "type": "markdown", "text": "" } })]).contains("id"));
        assert!(err(vec![markdown("a", "x"), markdown("a", "y")]).contains("twice"));
        assert!(err(vec![json!({ "id": "a", "definition": { "type": "markdown" } })]).contains("text"));
        assert!(err(vec![json!({ "id": "a", "definition": { "type": "timeseries", "requests": [] } })]).contains("query"));
        assert!(err(vec![json!({ "id": "a", "definition": { "type": "query_value", "request": { "query": " " } } })]).contains("query"));
        assert!(err(vec![json!({ "id": "a", "definition": { "type": "log_stream", "query": "", "limit": 0 } })]).contains("limit"));
        assert!(err(vec![json!({ "id": "a", "definition": { "type": "markdown", "text": "x" }, "time": "3y" })]).contains("time"));
        assert!(err(vec![json!("cell")]).contains("object"));
    }

    #[test]
    fn limits_the_size_of_a_notebook() {
        let many: Vec<Value> = (0..=MAX_CELLS).map(|i| markdown(&format!("c{i}"), "x")).collect();
        assert!(validate_cells(&many).unwrap_err().contains("at most"));
        let long = vec![markdown("a", &"x".repeat(MAX_MARKDOWN_CHARS + 1))];
        assert!(validate_cells(&long).unwrap_err().contains("characters"));
        let queries: Vec<Value> = (0..=MAX_REQUESTS).map(|_| json!({ "query": "avg:system.load.1{*}" })).collect();
        let wide = vec![json!({ "id": "a", "definition": { "type": "timeseries", "requests": queries } })];
        assert!(validate_cells(&wide).unwrap_err().contains("queries"));
    }

    #[test]
    fn labels_are_trimmed_deduplicated_and_bounded() {
        assert_eq!(clean_labels(vec![" payments ".into(), "".into(), "payments".into(), "checkout".into()], "teams"), Ok(vec!["payments".to_string(), "checkout".to_string()]));
        assert!(clean_labels((0..=MAX_LABELS).map(|i| format!("t{i}")).collect(), "tags").unwrap_err().contains("at most"));
        assert!(clean_labels(vec!["x".repeat(MAX_LABEL_CHARS + 1)], "tags").is_err());
        assert!(check_title("  ").is_err());
        assert_eq!(check_title(" Checkout latency "), Ok("Checkout latency".to_string()));
    }

    fn request(view: NotebookListView) -> NotebookSearchRequest {
        NotebookSearchRequest { query: String::new(), view, author: None, team: None, sort: NotebookSortField::Modified, dir: SortDir::Desc, offset: 0, limit: 50 }
    }

    const VIEWS: [NotebookListView; 6] = [NotebookListView::All, NotebookListView::Mine, NotebookListView::Team, NotebookListView::Recents, NotebookListView::Favorites, NotebookListView::Deleted];

    /// Every list is the org's notebooks; the per-user tables join on the org too and only for this user.
    #[test]
    fn every_list_is_scoped_to_the_org_and_the_user() {
        for view in VIEWS {
            let req = request(view);
            let sql = search_query("org-a", "user-a", &req).into_sql();
            assert!(sql.contains("WHERE n.org_id = $3"), "{view:?}: {sql}");
            assert!(sql.contains("v.org_id = n.org_id AND v.notebook_id = n.id AND v.user_id = $1::uuid"), "{view:?}");
            assert!(sql.contains("f.org_id = n.org_id AND f.notebook_id = n.id AND f.user_id = $2::uuid"), "{view:?}");
            assert!(!sql.contains("org-a") && !sql.contains("user-a"), "values are bound, never interpolated");
        }
    }

    #[test]
    fn lists_select_their_notebooks() {
        let sql = |view| search_query("o", "u", &request(view)).into_sql();
        assert!(sql(NotebookListView::All).contains("n.deleted_at IS NULL"));
        assert!(sql(NotebookListView::Deleted).contains("n.deleted_at IS NOT NULL"));
        assert!(sql(NotebookListView::Mine).contains("n.author_id = $4::uuid"));
        assert!(sql(NotebookListView::Team).contains("FROM team_members tm WHERE tm.org_id = n.org_id AND tm.user_id = $4::uuid"));
        assert!(sql(NotebookListView::Favorites).contains("f.user_id IS NOT NULL"));
        let recents = sql(NotebookListView::Recents);
        assert!(recents.contains("v.viewed_at IS NOT NULL") && recents.contains("ORDER BY v.viewed_at DESC"));
        for view in VIEWS.into_iter().filter(|v| *v != NotebookListView::Deleted) {
            assert!(sql(view).contains("n.deleted_at IS NULL"), "{view:?} hides deleted notebooks");
        }
    }

    #[test]
    fn text_author_and_team_filters_are_bound() {
        let mut req = request(NotebookListView::All);
        req.query = "50%_off".into();
        req.author = Some("00000000-0000-0000-0000-000000000001".into());
        req.team = Some("payments".into());
        let sql = search_query("o", "u", &req).into_sql();
        assert!(sql.contains("n.author_id::text = $4") && sql.contains("$5 = ANY(n.teams)") && sql.contains("n.cells::text ILIKE $9"));
        assert!(!sql.contains("payments") && !sql.contains("50%"));
    }

    #[test]
    fn soft_delete_and_restore_stay_in_the_org() {
        for sql in [SOFT_DELETE, RESTORE] {
            assert!(sql.contains("WHERE org_id = $1 AND id = $2"));
        }
        assert!(SOFT_DELETE.contains("SET deleted_at = now()") && SOFT_DELETE.contains("deleted_at IS NULL"));
        assert!(RESTORE.contains("SET deleted_at = NULL") && RESTORE.contains("deleted_at IS NOT NULL"));
    }
}
