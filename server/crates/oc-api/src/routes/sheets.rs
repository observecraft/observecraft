//! Dashboards → Sheets: spreadsheets whose tabs are backed by a data source query (metrics, logs or
//! a reference table) plus calculated columns, sort, filters and a pivot.
//!
//! The definition is the JSON the web app edits (`web/src/api/sheets/types.ts`): the web app runs
//! the queries through the existing metric, log and reference table APIs and evaluates formulas
//! itself. The server stores it per org with its author and team, and checks only its shape and
//! size. Every query is scoped by org.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::dashboards::UserRef;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_SHEETS: usize = 20;
const MAX_DEFINITION_BYTES: usize = 500_000;
const MAX_NAME_CHARS: usize = 200;
const MAX_TEAM_CHARS: usize = 100;
const SOURCES: &[&str] = &["metrics", "logs", "reference_table", "none"];

/// Checks a spreadsheet's name.
pub(crate) fn check_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME_CHARS {
        return Err(format!("The name must be 1–{MAX_NAME_CHARS} characters"));
    }
    Ok(n.to_string())
}

/// A team handle, or none.
pub(crate) fn check_team(team: Option<&str>) -> Result<Option<String>, String> {
    let Some(t) = team.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    if t.chars().count() > MAX_TEAM_CHARS {
        return Err(format!("A team is at most {MAX_TEAM_CHARS} characters"));
    }
    Ok(Some(t.to_string()))
}

/// Checks the definition: `{ sheets: [{ id, name, source: { type }, … }] }` within the limits.
pub(crate) fn validate_definition(def: &Value) -> Result<(), String> {
    let Some(sheets) = def.get("sheets").and_then(Value::as_array) else {
        return Err("A spreadsheet definition has a list of sheets".into());
    };
    if sheets.len() > MAX_SHEETS {
        return Err(format!("A spreadsheet has at most {MAX_SHEETS} sheets"));
    }
    if serde_json::to_string(def).map_or(0, |s| s.len()) > MAX_DEFINITION_BYTES {
        return Err(format!("A spreadsheet takes at most {} KB", MAX_DEFINITION_BYTES / 1000));
    }
    for (i, s) in sheets.iter().enumerate() {
        let n = i + 1;
        if s.get("id").and_then(Value::as_str).is_none_or(str::is_empty) {
            return Err(format!("Sheet {n} needs an id"));
        }
        let name = s.get("name").and_then(Value::as_str).unwrap_or_default();
        if name.trim().is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("Sheet {n} needs a name of 1–{MAX_NAME_CHARS} characters"));
        }
        let kind = s.pointer("/source/type").and_then(Value::as_str).unwrap_or_default();
        if !SOURCES.contains(&kind) {
            return Err(format!("Sheet {n}: unknown data source '{kind}' (use {})", SOURCES.join(", ")));
        }
    }
    Ok(())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetSummary {
    pub id: String,
    pub name: String,
    pub author: Option<UserRef>,
    pub team: Option<String>,
    pub sheet_count: i32,
    pub created_at: i64,
    pub modified_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Spreadsheet {
    pub id: String,
    pub name: String,
    pub author: Option<UserRef>,
    pub team: Option<String>,
    pub sheet_count: i32,
    pub created_at: i64,
    pub modified_at: i64,
    /// `{ sheets: [...] }` (schema owned by the web app).
    #[schema(value_type = Object)]
    pub definition: Value,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    author_id: Option<String>,
    author_name: Option<String>,
    team: Option<String>,
    sheet_count: i32,
    created_at: i64,
    modified_at: i64,
    definition: Value,
}

impl Row {
    fn author(&self) -> Option<UserRef> {
        self.author_id.clone().map(|id| UserRef { id, name: self.author_name.clone().unwrap_or_default() })
    }
    fn summary(self) -> SpreadsheetSummary {
        SpreadsheetSummary { author: self.author(), id: self.id, name: self.name, team: self.team, sheet_count: self.sheet_count, created_at: self.created_at, modified_at: self.modified_at }
    }
    fn full(self) -> Spreadsheet {
        Spreadsheet {
            author: self.author(),
            id: self.id,
            name: self.name,
            team: self.team,
            sheet_count: self.sheet_count,
            created_at: self.created_at,
            modified_at: self.modified_at,
            definition: self.definition,
        }
    }
}

const SELECT: &str = "SELECT s.id::text, s.name, u.id::text AS author_id, u.name AS author_name, s.team, \
    COALESCE(jsonb_array_length(s.definition -> 'sheets'), 0)::int AS sheet_count, \
    (extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM s.modified_at) * 1000)::bigint AS modified_at, s.definition \
    FROM spreadsheets s LEFT JOIN users u ON u.id = s.author_id WHERE s.org_id = $1";

async fn load(st: &AppState, org: &str, id: &str) -> Result<Spreadsheet, ApiError> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} AND s.id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(Row::full).ok_or_else(|| ApiError::NotFound(format!("Spreadsheet {id} not found")))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetSearchRequest {
    #[serde(default)]
    pub query: String,
    /// all | mine
    #[serde(default)]
    pub view: String,
    pub team: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetSearchPage {
    pub spreadsheets: Vec<SpreadsheetSummary>,
    pub total: i64,
    /// Teams with at least one spreadsheet, for the Teams filter.
    pub teams: Vec<String>,
}

/// Spreadsheets by name (or author), all or the current user's, optionally of one team; newest first.
#[utoipa::path(post, path = "/api/v1/spreadsheets/search", operation_id = "search_spreadsheets", request_body = SpreadsheetSearchRequest, responses((status = 200, body = SpreadsheetSearchPage)), tag = "dashboards")]
pub async fn search(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SpreadsheetSearchRequest>) -> ApiResult<SpreadsheetSearchPage> {
    let pattern = super::dashboards::ilike(req.query.trim());
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "{SELECT} AND (s.name ILIKE $2 OR COALESCE(u.name, '') ILIKE $2) AND ($3 <> 'mine' OR s.author_id = $4::uuid) AND ($5::text IS NULL OR s.team = $5) ORDER BY s.modified_at DESC LIMIT 500"
    ))
    .bind(&ctx.org_id)
    .bind(&pattern)
    .bind(&req.view)
    .bind(&ctx.user_id)
    .bind(req.team.as_deref().filter(|t| !t.is_empty()))
    .fetch_all(&st.postgres)
    .await?;
    let teams: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT team FROM spreadsheets WHERE org_id = $1 AND team IS NOT NULL ORDER BY team").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(SpreadsheetSearchPage { total: rows.len() as i64, spreadsheets: rows.into_iter().map(Row::summary).collect(), teams: teams.into_iter().map(|(t,)| t).collect() }))
}

#[utoipa::path(get, path = "/api/v1/spreadsheets/{id}", operation_id = "get_spreadsheet", params(("id" = String, Path)), responses((status = 200, body = Spreadsheet), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Spreadsheet> {
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateSpreadsheetRequest {
    pub name: String,
    pub team: Option<String>,
    /// Defaults to one empty sheet.
    #[schema(value_type = Option<Object>)]
    pub definition: Option<Value>,
}

#[utoipa::path(post, path = "/api/v1/spreadsheets", operation_id = "create_spreadsheet", request_body = CreateSpreadsheetRequest, responses((status = 200, body = Spreadsheet), (status = 400, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<CreateSpreadsheetRequest>) -> ApiResult<Spreadsheet> {
    let name = check_name(&req.name).map_err(ApiError::BadRequest)?;
    let team = check_team(req.team.as_deref()).map_err(ApiError::BadRequest)?;
    let def = req.definition.unwrap_or_else(|| json!({ "sheets": [{ "id": "sheet1", "name": "Sheet 1", "source": { "type": "none" }, "columns": [] }] }));
    validate_definition(&def).map_err(ApiError::BadRequest)?;
    let (id,): (String,) = sqlx::query_as("INSERT INTO spreadsheets (org_id, name, team, author_id, definition) VALUES ($1, $2, $3, $4::uuid, $5) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&name)
        .bind(&team)
        .bind(&ctx.user_id)
        .bind(&def)
        .fetch_one(&st.postgres)
        .await?;
    ctx.audit(&st, "spreadsheet.created", &name, &id).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetPatch {
    pub name: Option<String>,
    /// An empty string clears the team.
    pub team: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub definition: Option<Value>,
}

#[utoipa::path(patch, path = "/api/v1/spreadsheets/{id}", operation_id = "update_spreadsheet", params(("id" = String, Path)), request_body = SpreadsheetPatch, responses((status = 200, body = Spreadsheet), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(p): Json<SpreadsheetPatch>) -> ApiResult<Spreadsheet> {
    let name = p.name.as_deref().map(check_name).transpose().map_err(ApiError::BadRequest)?;
    let team = match p.team.as_deref() {
        None => None,
        Some(t) => Some(check_team(Some(t)).map_err(ApiError::BadRequest)?),
    };
    if let Some(d) = &p.definition {
        validate_definition(d).map_err(ApiError::BadRequest)?;
    }
    let updated = sqlx::query(
        "UPDATE spreadsheets SET name = COALESCE($3, name), team = CASE WHEN $4 THEN $5 ELSE team END, definition = COALESCE($6, definition), modified_at = now() \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&name)
    .bind(team.is_some())
    .bind(team.flatten())
    .bind(&p.definition)
    .execute(&st.postgres)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Spreadsheet {id} not found")));
    }
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/spreadsheets/{id}", operation_id = "delete_spreadsheet", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "dashboards")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM spreadsheets WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound(format!("Spreadsheet {id} not found"))) };
    ctx.audit(&st, "spreadsheet.deleted", &name, &id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_teams() {
        assert_eq!(check_name("  Orders by region ").unwrap(), "Orders by region");
        assert!(check_name(" ").is_err());
        assert!(check_name(&"x".repeat(201)).is_err());
        assert_eq!(check_team(Some(" checkout ")).unwrap(), Some("checkout".into()));
        assert_eq!(check_team(Some("")).unwrap(), None);
        assert_eq!(check_team(None).unwrap(), None);
    }

    #[test]
    fn definitions_are_checked() {
        let ok = json!({ "sheets": [{ "id": "s1", "name": "Latency", "source": { "type": "metrics", "query": "avg:trace.http.request.duration{*} by {service}" } }] });
        assert!(validate_definition(&ok).is_ok());
        assert!(validate_definition(&json!({})).is_err());
        assert!(validate_definition(&json!({ "sheets": [{ "id": "s1", "name": "", "source": { "type": "metrics" } }] })).is_err());
        assert!(validate_definition(&json!({ "sheets": [{ "name": "A", "source": { "type": "metrics" } }] })).is_err());
        let e = validate_definition(&json!({ "sheets": [{ "id": "s1", "name": "A", "source": { "type": "sql" } }] })).unwrap_err();
        assert!(e.contains("unknown data source 'sql'"), "{e}");
        let many: Vec<Value> = (0..21).map(|i| json!({ "id": format!("s{i}"), "name": "A", "source": { "type": "none" } })).collect();
        assert!(validate_definition(&json!({ "sheets": many })).is_err());
    }
}
