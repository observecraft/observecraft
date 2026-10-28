//! Log data access (Logs → Configuration → Data Access): admin-only CRUD of the org's log
//! restriction queries and their role and team assignments. What they do is in `crate::log_access`.
//! Every change is audited and applies at once on this server (the cache is cleared), within 10 s
//! on the others.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::identity::{RoleRef, TeamRef};
use super::logs::{bind, scope, TimeRange};
use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::log_access::{validate_query, LogAccess};
use crate::state::AppState;

const MAX_QUERIES: i64 = 100;
const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;
/// Roles, and teams, per restriction query.
const MAX_ASSIGNMENTS: usize = 50;
const HOUR_MS: i64 = 3_600_000;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogRestrictionQuery {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Log search syntax; assigned users read only the logs matching it (or another of their queries).
    pub query: String,
    pub roles: Vec<RoleRef>,
    pub teams: Vec<TeamRef>,
    /// Name of the user who created it.
    pub created_by: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestrictionQueryInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub query: String,
    /// Roles whose users read the matching logs (not the admin role: admins read everything).
    #[serde(default)]
    pub role_ids: Vec<String>,
    /// Teams whose (direct) members read the matching logs.
    #[serde(default)]
    pub team_handles: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestrictionRoleInput {
    pub role_id: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RestrictionTeamInput {
    pub handle: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RestrictionPreviewRequest {
    pub query: String,
}

#[derive(Serialize, ToSchema)]
pub struct RestrictionPreview {
    /// Logs of the last hour matching the query.
    pub count: u64,
}

fn dedup(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    values.retain(|v| !v.is_empty() && seen.insert(v.clone()));
}

/// Trims and checks an input; `org` names the admin role, which can't be restricted.
fn validate(input: &mut RestrictionQueryInput, org: &str) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    input.description = input.description.trim().to_string();
    input.query = input.query.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Name must be 1–{MAX_NAME_CHARS} characters"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    validate_query(&input.query)?;
    for v in input.role_ids.iter_mut().chain(input.team_handles.iter_mut()) {
        *v = v.trim().to_string();
    }
    dedup(&mut input.role_ids);
    dedup(&mut input.team_handles);
    if input.role_ids.len() > MAX_ASSIGNMENTS || input.team_handles.len() > MAX_ASSIGNMENTS {
        return Err(format!("At most {MAX_ASSIGNMENTS} roles and {MAX_ASSIGNMENTS} teams per query"));
    }
    if input.role_ids.iter().any(|r| *r == admin_role(org)) {
        return Err("Admins always read every log: the admin role can't be restricted".into());
    }
    Ok(())
}

fn admin_role(org: &str) -> String {
    format!("{org}-admin")
}

/// The hour before `now` (epoch ms).
fn preview_range(now: i64) -> TimeRange {
    TimeRange { from: now - HOUR_MS, to: now }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    description: String,
    query: String,
    created_by: Option<String>,
    created_at: i64,
    updated_at: i64,
    roles: Value,
    teams: Value,
}

impl From<Row> for LogRestrictionQuery {
    fn from(r: Row) -> Self {
        LogRestrictionQuery {
            id: r.id,
            name: r.name,
            description: r.description,
            query: r.query,
            roles: serde_json::from_value::<Vec<NamedRole>>(r.roles).unwrap_or_default().into_iter().map(|x| RoleRef { id: x.id, name: x.name }).collect(),
            teams: serde_json::from_value::<Vec<NamedTeam>>(r.teams).unwrap_or_default().into_iter().map(|x| TeamRef { handle: x.handle, name: x.name }).collect(),
            created_by: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize)]
struct NamedRole {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct NamedTeam {
    handle: String,
    name: String,
}

const SELECT: &str = "SELECT q.id::text AS id, q.name, q.description, q.query, u.name AS created_by, \
    (extract(epoch FROM q.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM q.updated_at) * 1000)::bigint AS updated_at, \
    COALESCE((SELECT json_agg(json_build_object('id', r.id, 'name', r.name) ORDER BY r.name) FROM log_restriction_roles a \
      JOIN roles r ON r.org_id = a.org_id AND r.id = a.role_id WHERE a.org_id = q.org_id AND a.restriction_id = q.id), '[]'::json) AS roles, \
    COALESCE((SELECT json_agg(json_build_object('handle', t.handle, 'name', t.name) ORDER BY t.name) FROM log_restriction_teams a \
      JOIN teams t ON t.org_id = a.org_id AND t.handle = a.team_handle WHERE a.org_id = q.org_id AND a.restriction_id = q.id), '[]'::json) AS teams \
    FROM log_restriction_queries q LEFT JOIN users u ON u.id = q.created_by";

async fn get_query(st: &AppState, org: &str, id: &str) -> Result<LogRestrictionQuery, ApiError> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE q.org_id = $1 AND q.id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(Into::into).ok_or_else(|| ApiError::NotFound("Restriction query not found".into()))
}

const ORG_ROLES: &str = "SELECT id FROM roles WHERE org_id = $1 AND id = ANY($2)";
const ORG_TEAMS: &str = "SELECT handle FROM teams WHERE org_id = $1 AND handle = ANY($2)";

/// The ids among `ids` that `existing` (`ORG_ROLES` or `ORG_TEAMS`) doesn't find in the org.
async fn unknown(st: &AppState, org: &str, existing: &str, ids: &[String]) -> Result<Vec<String>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let found: Vec<String> = sqlx::query_scalar(existing).bind(org).bind(ids).fetch_all(&st.postgres).await?;
    Ok(ids.iter().filter(|i| !found.contains(i)).cloned().collect())
}

/// Every role and team of the input exists in the org.
async fn check_assignments(st: &AppState, org: &str, input: &RestrictionQueryInput) -> Result<(), ApiError> {
    let roles = unknown(st, org, ORG_ROLES, &input.role_ids).await?;
    if !roles.is_empty() {
        return Err(ApiError::BadRequest(format!("Unknown role: {}", roles.join(", "))));
    }
    let teams = unknown(st, org, ORG_TEAMS, &input.team_handles).await?;
    if !teams.is_empty() {
        return Err(ApiError::BadRequest(format!("Unknown team: {}", teams.join(", "))));
    }
    Ok(())
}

/// Replaces the query's assignments.
async fn set_assignments(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: &str, id: &str, input: &RestrictionQueryInput) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM log_restriction_roles WHERE org_id = $1 AND restriction_id = $2::uuid").bind(org).bind(id).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM log_restriction_teams WHERE org_id = $1 AND restriction_id = $2::uuid").bind(org).bind(id).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO log_restriction_roles (org_id, restriction_id, role_id) SELECT $1, $2::uuid, unnest($3::text[])").bind(org).bind(id).bind(&input.role_ids).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO log_restriction_teams (org_id, restriction_id, team_handle) SELECT $1, $2::uuid, unnest($3::text[])")
        .bind(org)
        .bind(id)
        .bind(&input.team_handles)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A restriction query named '{name}' already exists")),
        _ => e.into(),
    }
}

/// Clears the cache and audits a change.
async fn changed(st: &AppState, ctx: &AuthContext, action: &str, target: &str, detail: &str) {
    st.log_restrictions.invalidate(&ctx.org_id);
    ctx.audit(st, action, target, detail).await;
}

fn assignment_detail(input: &RestrictionQueryInput) -> String {
    format!("query: {}; roles: {}; teams: {}", input.query, input.role_ids.join(", "), input.team_handles.join(", "))
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/logs/restriction-queries", operation_id = "list_log_restriction_queries", responses((status = 200, body = Vec<LogRestrictionQuery>)), tag = "logs")]
pub async fn list(State(st): State<AppState>, Admin(ctx): Admin) -> ApiResult<Vec<LogRestrictionQuery>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE q.org_id = $1 ORDER BY q.name")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[utoipa::path(post, path = "/api/v1/logs/restriction-queries", operation_id = "create_log_restriction_query", request_body = RestrictionQueryInput,
    responses((status = 200, body = LogRestrictionQuery), (status = 400, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Json(mut input): Json<RestrictionQueryInput>) -> ApiResult<LogRestrictionQuery> {
    validate(&mut input, &ctx.org_id).map_err(ApiError::BadRequest)?;
    check_assignments(&st, &ctx.org_id, &input).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_restriction_queries WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_QUERIES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_QUERIES} restriction queries")));
    }
    let mut tx = st.postgres.begin().await?;
    let (id,): (String,) = sqlx::query_as("INSERT INTO log_restriction_queries (org_id, name, description, query, created_by) VALUES ($1, $2, $3, $4, $5::uuid) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.query)
        .bind(&ctx.user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| unique(e, &input.name))?;
    set_assignments(&mut tx, &ctx.org_id, &id, &input).await?;
    tx.commit().await?;
    changed(&st, &ctx, "log_restriction.created", &input.name, &assignment_detail(&input)).await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

/// Replaces the name, description, query and assignments.
#[utoipa::path(put, path = "/api/v1/logs/restriction-queries/{id}", operation_id = "update_log_restriction_query", params(("id" = String, Path)), request_body = RestrictionQueryInput,
    responses((status = 200, body = LogRestrictionQuery), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn update(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(mut input): Json<RestrictionQueryInput>) -> ApiResult<LogRestrictionQuery> {
    validate(&mut input, &ctx.org_id).map_err(ApiError::BadRequest)?;
    get_query(&st, &ctx.org_id, &id).await?;
    check_assignments(&st, &ctx.org_id, &input).await?;
    let mut tx = st.postgres.begin().await?;
    sqlx::query("UPDATE log_restriction_queries SET name = $3, description = $4, query = $5, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.query)
        .execute(&mut *tx)
        .await
        .map_err(|e| unique(e, &input.name))?;
    set_assignments(&mut tx, &ctx.org_id, &id, &input).await?;
    tx.commit().await?;
    changed(&st, &ctx, "log_restriction.updated", &input.name, &assignment_detail(&input)).await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/logs/restriction-queries/{id}", operation_id = "delete_log_restriction_query", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn delete(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String, String)> =
        sqlx::query_as("DELETE FROM log_restriction_queries WHERE org_id = $1 AND id::text = $2 RETURNING name, query").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name, query)) = row else { return Err(ApiError::NotFound("Restriction query not found".into())) };
    changed(&st, &ctx, "log_restriction.deleted", &name, &query).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/logs/restriction-queries/{id}/roles", operation_id = "assign_log_restriction_role", params(("id" = String, Path)), request_body = RestrictionRoleInput,
    responses((status = 200, body = LogRestrictionQuery), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn assign_role(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(r): Json<RestrictionRoleInput>) -> ApiResult<LogRestrictionQuery> {
    let q = get_query(&st, &ctx.org_id, &id).await?;
    let role = r.role_id.trim().to_string();
    if role == admin_role(&ctx.org_id) {
        return Err(ApiError::BadRequest("Admins always read every log: the admin role can't be restricted".into()));
    }
    if !unknown(&st, &ctx.org_id, ORG_ROLES, std::slice::from_ref(&role)).await?.is_empty() {
        return Err(ApiError::BadRequest(format!("Unknown role: {role}")));
    }
    if q.roles.len() >= MAX_ASSIGNMENTS {
        return Err(ApiError::BadRequest(format!("At most {MAX_ASSIGNMENTS} roles per query")));
    }
    sqlx::query("INSERT INTO log_restriction_roles (org_id, restriction_id, role_id) VALUES ($1, $2::uuid, $3) ON CONFLICT DO NOTHING")
        .bind(&ctx.org_id)
        .bind(&q.id)
        .bind(&role)
        .execute(&st.postgres)
        .await?;
    changed(&st, &ctx, "log_restriction.role_assigned", &q.name, &role).await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/logs/restriction-queries/{id}/roles/{role_id}", operation_id = "unassign_log_restriction_role", params(("id" = String, Path), ("role_id" = String, Path)),
    responses((status = 200, body = LogRestrictionQuery), (status = 404, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn unassign_role(State(st): State<AppState>, Admin(ctx): Admin, Path((id, role)): Path<(String, String)>) -> ApiResult<LogRestrictionQuery> {
    let q = get_query(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_restriction_roles WHERE org_id = $1 AND restriction_id = $2::uuid AND role_id = $3").bind(&ctx.org_id).bind(&q.id).bind(&role).execute(&st.postgres).await?;
    changed(&st, &ctx, "log_restriction.role_unassigned", &q.name, &role).await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/logs/restriction-queries/{id}/teams", operation_id = "assign_log_restriction_team", params(("id" = String, Path)), request_body = RestrictionTeamInput,
    responses((status = 200, body = LogRestrictionQuery), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn assign_team(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(t): Json<RestrictionTeamInput>) -> ApiResult<LogRestrictionQuery> {
    let q = get_query(&st, &ctx.org_id, &id).await?;
    let handle = t.handle.trim().to_string();
    if !unknown(&st, &ctx.org_id, ORG_TEAMS, std::slice::from_ref(&handle)).await?.is_empty() {
        return Err(ApiError::BadRequest(format!("Unknown team: {handle}")));
    }
    if q.teams.len() >= MAX_ASSIGNMENTS {
        return Err(ApiError::BadRequest(format!("At most {MAX_ASSIGNMENTS} teams per query")));
    }
    sqlx::query("INSERT INTO log_restriction_teams (org_id, restriction_id, team_handle) VALUES ($1, $2::uuid, $3) ON CONFLICT DO NOTHING")
        .bind(&ctx.org_id)
        .bind(&q.id)
        .bind(&handle)
        .execute(&st.postgres)
        .await?;
    changed(&st, &ctx, "log_restriction.team_assigned", &q.name, &handle).await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/logs/restriction-queries/{id}/teams/{handle}", operation_id = "unassign_log_restriction_team", params(("id" = String, Path), ("handle" = String, Path)),
    responses((status = 200, body = LogRestrictionQuery), (status = 404, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn unassign_team(State(st): State<AppState>, Admin(ctx): Admin, Path((id, handle)): Path<(String, String)>) -> ApiResult<LogRestrictionQuery> {
    let q = get_query(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_restriction_teams WHERE org_id = $1 AND restriction_id = $2::uuid AND team_handle = $3").bind(&ctx.org_id).bind(&q.id).bind(&handle).execute(&st.postgres).await?;
    changed(&st, &ctx, "log_restriction.team_unassigned", &q.name, &handle).await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

/// Checks a query and counts the logs of the last hour it matches. Admins read every log, so the
/// count is the query's alone.
#[utoipa::path(post, path = "/api/v1/logs/restriction-queries/preview", operation_id = "preview_log_restriction_query", request_body = RestrictionPreviewRequest,
    responses((status = 200, body = RestrictionPreview), (status = 400, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn preview(State(st): State<AppState>, Admin(ctx): Admin, access: LogAccess, Json(req): Json<RestrictionPreviewRequest>) -> ApiResult<RestrictionPreview> {
    let terms = validate_query(&req.query).map_err(ApiError::BadRequest)?;
    let s = scope(&ctx.org_id, &access, preview_range(now_ms()), &terms, None);
    let count = bind(st.ch(&ctx.org_id).query(&format!("SELECT count() FROM {} WHERE {}", s.table, s.clause)), &s.params).fetch_one::<u64>().await?;
    Ok(Json(RestrictionPreview { count }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, query: &str, roles: &[&str], teams: &[&str]) -> RestrictionQueryInput {
        RestrictionQueryInput {
            name: name.into(),
            description: String::new(),
            query: query.into(),
            role_ids: roles.iter().map(|r| r.to_string()).collect(),
            team_handles: teams.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn inputs_are_trimmed_and_deduplicated() {
        let mut i = input("  Billing team ", " service:billing ", &["r1", "r1", "org-standard"], &["payments", "payments"]);
        assert_eq!(validate(&mut i, "org"), Ok(()));
        assert_eq!((i.name.as_str(), i.query.as_str()), ("Billing team", "service:billing"));
        assert_eq!(i.role_ids, vec!["r1".to_string(), "org-standard".to_string()]);
        assert_eq!(i.team_handles, vec!["payments".to_string()]);
    }

    #[test]
    fn names_and_queries_are_required() {
        assert!(validate(&mut input("", "service:billing", &[], &[]), "org").is_err());
        assert!(validate(&mut input(&"x".repeat(MAX_NAME_CHARS + 1), "service:billing", &[], &[]), "org").is_err());
        assert!(validate(&mut input("Billing", "", &[], &[]), "org").is_err());
        assert!(validate(&mut input("Billing", "index:main", &[], &[]), "org").is_err());
        let mut long = input("Billing", "service:billing", &[], &[]);
        long.description = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert!(validate(&mut long, "org").is_err());
    }

    #[test]
    fn the_admin_role_cant_be_restricted() {
        let err = validate(&mut input("Billing", "service:billing", &["org-admin"], &[]), "org").unwrap_err();
        assert!(err.contains("admin"), "{err}");
        // Another org's admin role id is just an unknown role here (rejected when looked up).
        assert_eq!(validate(&mut input("Billing", "service:billing", &["other-admin"], &[]), "org"), Ok(()));
    }

    #[test]
    fn assignments_are_bounded() {
        let many: Vec<String> = (0..=MAX_ASSIGNMENTS).map(|i| format!("r{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(validate(&mut input("Billing", "service:billing", &refs, &[]), "org").is_err());
        assert!(validate(&mut input("Billing", "service:billing", &[], &refs), "org").is_err());
    }

    #[test]
    fn previews_count_the_last_hour() {
        let r = preview_range(10 * 3_600_000);
        assert_eq!((r.from, r.to), (9 * 3_600_000, 10 * 3_600_000));
    }
}
