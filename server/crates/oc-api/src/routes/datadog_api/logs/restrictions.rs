//! Log restriction queries, Datadog-compatible (`/api/v2/logs/config/restriction_queries`): the
//! provider's `datadog_logs_restriction_query`, on `log_restriction_queries` and its role assignments
//! ([`crate::routes::log_restrictions`], [`crate::log_access`]).
//!
//! - Datadog's queries have no name: ours are named after the query (`(2)`… when two are the same),
//!   and renamed with it while the name is still the query.
//! - Team assignments (ours only) are kept and not shown here.
//! - Admins only, as in our API; every change clears the restriction cache.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::super::users::{bad_request, body, data, iso, not_found, ok, relationship_id, require, uuid, DocResult};
use super::super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::log_access::validate_query;
use crate::state::AppState;

const MAX_QUERIES: i64 = 100;

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    query: String,
    created_at: i64,
    updated_at: i64,
    roles: Vec<String>,
    user_count: i64,
}

const SELECT: &str = "SELECT q.id::text AS id, q.name, q.query, (extract(epoch FROM q.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM q.updated_at) * 1000)::bigint AS updated_at, \
    COALESCE(ARRAY(SELECT a.role_id FROM log_restriction_roles a WHERE a.org_id = q.org_id AND a.restriction_id = q.id ORDER BY a.role_id), '{}') AS roles, \
    (SELECT count(DISTINCT ur.user_id) FROM log_restriction_roles a JOIN user_roles ur ON ur.role_id = a.role_id WHERE a.org_id = q.org_id AND a.restriction_id = q.id) AS user_count \
    FROM log_restriction_queries q";

fn doc(r: &Row) -> Value {
    json!({
        "type": "logs_restriction_queries",
        "id": r.id,
        "attributes": {
            "restriction_query": r.query,
            "created_at": iso(r.created_at),
            "modified_at": iso(r.updated_at),
            "role_count": r.roles.len(),
            "user_count": r.user_count,
        },
        "relationships": { "roles": { "data": r.roles.iter().map(|id| json!({ "type": "roles", "id": id })).collect::<Vec<_>>() } },
    })
}

async fn find(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    let Some(id) = uuid(id) else { return Err(bad_request(format!("Invalid restriction query id {id}"))) };
    sqlx::query_as(&format!("{SELECT} WHERE q.org_id = $1 AND q.id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?.ok_or_else(|| not_found("Restriction query not found"))
}

/// The `restriction_query` of a request.
fn query_of(raw: &Bytes) -> Result<String, DdError> {
    let v = body(raw)?;
    let d = v.get("data").ok_or_else(|| bad_request("API input validation failed: Object must include `data` key."))?;
    let q = d.get("attributes").and_then(|a| a.get("restriction_query")).and_then(Value::as_str).ok_or_else(|| bad_request("The parameter 'restriction_query' is required"))?.trim().to_string();
    validate_query(&q).map_err(bad_request)?;
    Ok(q)
}

/// A free name for a query: the query itself (shortened), then `… (2)`, `… (3)`.
async fn free_name(st: &AppState, org: &str, query: &str, except: Option<&str>) -> Result<String, DdError> {
    let base: String = query.chars().take(90).collect();
    for n in 1..1000 {
        let name = if n == 1 { base.clone() } else { format!("{base} ({n})") };
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM log_restriction_queries WHERE org_id = $1 AND name = $2 AND id::text <> COALESCE($3, ''))")
            .bind(org)
            .bind(&name)
            .bind(except)
            .fetch_one(&st.postgres)
            .await?;
        if !taken {
            return Ok(name);
        }
    }
    Err(bad_request("Too many restriction queries with this query"))
}

async fn changed(st: &AppState, ctx: &AuthContext, action: &str, target: &str) {
    st.log_restrictions.invalidate(&ctx.org_id);
    ctx.audit(st, action, target, "API").await;
}

fn page(rows: Vec<Row>, q: &HashMap<String, String>) -> Vec<Value> {
    let size = q.get("page[size]").and_then(|s| s.parse::<usize>().ok()).unwrap_or(10).clamp(1, 1000);
    let number = q.get("page[number]").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    rows.iter().skip(size * number).take(size).map(doc).collect()
}

pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE q.org_id = $1 ORDER BY q.created_at")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    ok(json!({ "data": page(rows, &q) }))
}

pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let r = find(&st, &ctx.org_id, &id).await?;
    let included = super::super::roles::role_docs(&st, &ctx.org_id, &r.roles).await?;
    ok(json!({ "data": doc(&r), "included": included }))
}

pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let query = query_of(&raw)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_restriction_queries WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_QUERIES {
        return Err(bad_request(format!("An organization can have at most {MAX_QUERIES} restriction queries")));
    }
    let name = free_name(&st, &ctx.org_id, &query, None).await?;
    let (id,): (String,) = sqlx::query_as("INSERT INTO log_restriction_queries (org_id, name, query, created_by) VALUES ($1, $2, $3, $4::uuid) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&name)
        .bind(&query)
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await?;
    changed(&st, &ctx, "log_restriction.created", &name).await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &id).await?) }))
}

async fn set_query(st: &AppState, ctx: &DdAuth, id: &str, raw: &Bytes) -> DocResult {
    let ctx = &ctx.0;
    require(ctx, Access::Admin)?;
    let current = find(st, &ctx.org_id, id).await?;
    let query = query_of(raw)?;
    let name = if current.name.starts_with(&current.query.chars().take(90).collect::<String>()) { free_name(st, &ctx.org_id, &query, Some(&current.id)).await? } else { current.name.clone() };
    sqlx::query("UPDATE log_restriction_queries SET query = $3, name = $4, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&current.id)
        .bind(&query)
        .bind(&name)
        .execute(&st.postgres)
        .await?;
    changed(st, ctx, "log_restriction.updated", &name).await;
    ok(json!({ "data": doc(&find(st, &ctx.org_id, &current.id).await?) }))
}

pub async fn update(State(st): State<AppState>, auth: DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    set_query(&st, &auth, &id, &raw).await
}

pub async fn replace(State(st): State<AppState>, auth: DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    set_query(&st, &auth, &id, &raw).await
}

pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let r = find(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_restriction_queries WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&r.id).execute(&st.postgres).await?;
    changed(&st, &ctx, "log_restriction.deleted", &r.name).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_roles(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let r = find(&st, &ctx.org_id, &id).await?;
    let names: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM roles WHERE org_id = $1 AND id = ANY($2) ORDER BY name").bind(&ctx.org_id).bind(&r.roles).fetch_all(&st.postgres).await?;
    ok(json!({ "data": names.iter().map(|(id, name)| json!({ "type": "roles", "id": id, "attributes": { "name": name } })).collect::<Vec<_>>() }))
}

/// The role a grant or revoke names, checked to be one of the org's (and not the admin role).
async fn role_of(st: &AppState, ctx: &AuthContext, raw: &Bytes) -> Result<String, DdError> {
    let v = body(raw)?;
    let role = relationship_id(data(&v)?, "roles")?;
    if role == format!("{}-admin", ctx.org_id) {
        return Err(bad_request("Admins always read every log: the admin role can't be restricted"));
    }
    if super::super::roles::find_role(st, &ctx.org_id, &role).await?.is_none() {
        return Err(not_found(format!("Role {role} not found")));
    }
    Ok(role)
}

pub async fn grant_role(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let r = find(&st, &ctx.org_id, &id).await?;
    let role = role_of(&st, &ctx, &raw).await?;
    // A role reads through one restriction query at a time in Datadog: granting moves it.
    let mut tx = st.postgres.begin().await?;
    sqlx::query("DELETE FROM log_restriction_roles WHERE org_id = $1 AND role_id = $2").bind(&ctx.org_id).bind(&role).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO log_restriction_roles (org_id, restriction_id, role_id) VALUES ($1, $2::uuid, $3)").bind(&ctx.org_id).bind(&r.id).bind(&role).execute(&mut *tx).await?;
    tx.commit().await?;
    changed(&st, &ctx, "log_restriction.role_assigned", &r.name).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_role(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let r = find(&st, &ctx.org_id, &id).await?;
    let role = role_of(&st, &ctx, &raw).await?;
    sqlx::query("DELETE FROM log_restriction_roles WHERE org_id = $1 AND restriction_id = $2::uuid AND role_id = $3").bind(&ctx.org_id).bind(&r.id).bind(&role).execute(&st.postgres).await?;
    changed(&st, &ctx, "log_restriction.role_unassigned", &r.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The queries that apply to a user (through their roles).
pub async fn for_user(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(user_id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let Some(uid) = uuid(&user_id) else { return Err(bad_request(format!("user with uuid {user_id} doesn't exist"))) };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE org_id = $1 AND id = $2::uuid)").bind(&ctx.org_id).bind(&uid).fetch_one(&st.postgres).await?;
    if !exists {
        return Err(bad_request(format!("user with uuid {user_id} doesn't exist")));
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "{SELECT} WHERE q.org_id = $1 AND q.id IN (SELECT a.restriction_id FROM log_restriction_roles a JOIN user_roles ur ON ur.role_id = a.role_id \
         WHERE a.org_id = $1 AND ur.user_id = $2::uuid) ORDER BY q.created_at"
    ))
    .bind(&ctx.org_id)
    .bind(&uid)
    .fetch_all(&st.postgres)
    .await?;
    ok(json!({ "data": rows.iter().map(doc).collect::<Vec<_>>() }))
}

/// The query a role reads through (none: an empty list).
pub async fn for_role(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(role_id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    if super::super::roles::find_role(&st, &ctx.org_id, &role_id).await?.is_none() {
        return Err(bad_request(format!("role with uuid {role_id} doesn't exist")));
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "{SELECT} WHERE q.org_id = $1 AND q.id IN (SELECT a.restriction_id FROM log_restriction_roles a WHERE a.org_id = $1 AND a.role_id = $2) ORDER BY q.created_at"
    ))
    .bind(&ctx.org_id)
    .bind(&role_id)
    .fetch_all(&st.postgres)
    .await?;
    ok(json!({ "data": rows.iter().map(doc).collect::<Vec<_>>() }))
}
