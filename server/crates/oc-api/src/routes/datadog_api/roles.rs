//! Roles and permissions, Datadog-compatible (`/api/v2/roles`, `/api/v2/permissions`), on our
//! `roles` / `user_roles` tables: the provider's `datadog_role` (with `permission` blocks) and
//! `datadog_user_role` resources, `datadog_role(s)` / `datadog_permissions` data sources, and the
//! role assignments `datadog_user` makes.
//!
//! - The built-in roles (Admin, Standard, Read Only) report the permissions of their access level
//!   ([`super::permissions`]) and can't be changed or deleted; Datadog's names for them
//!   ("Datadog Admin Role", …) find them in `filter`.
//! - A custom role holds exactly the permissions it was given (stored in `role_permissions`); its
//!   members' access still comes from the built-in roles they have.
//! - Unknown permission ids are rejected; nobody can give a role a permission they don't have.
//! - Only admins manage roles and assignments, as in our API; admins can't drop their own admin role.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder};

use super::permissions::{self as perms, Permission};
use super::users::{bad_request, body, check_name, data, ilike, not_found, ok, relationship_id, relationship_ids, require, user_doc, DdPage, DocResult, UserRow, USER_COLS};
use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
pub struct RoleRow {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub user_count: i64,
    pub stored: Vec<String>,
}

const ROLE_COLS: &str = "r.id, r.name, (extract(epoch FROM r.created_at) * 1000)::bigint AS created_at, \
    (SELECT count(*) FROM user_roles ur WHERE ur.role_id = r.id) AS user_count, \
    COALESCE((SELECT array_agg(p.permission) FROM role_permissions p WHERE p.org_id = r.org_id AND p.role_id = r.id), '{}') AS stored";

impl RoleRow {
    /// The role's permissions: its level's for a built-in role, else the ones it was given.
    pub fn permissions(&self, org: &str) -> Vec<&'static Permission> {
        match perms::builtin_level(org, &self.id) {
            Some(level) => perms::granted(level).collect(),
            None => perms::ordered(self.stored.iter().map(String::as_str)),
        }
    }
}

/// The role as a Datadog `roles` document.
pub fn role_doc(r: &RoleRow, org: &str) -> Value {
    let created = super::users::iso(r.created_at);
    let rels: Vec<Value> = r.permissions(org).into_iter().map(perms::relationship).collect();
    json!({
        "type": "roles",
        "id": r.id,
        "attributes": { "name": r.name, "created_at": created, "modified_at": created, "user_count": r.user_count },
        "relationships": { "permissions": { "data": rels } },
    })
}

pub async fn find_role(st: &AppState, org: &str, id: &str) -> Result<Option<RoleRow>, DdError> {
    Ok(sqlx::query_as(&format!("SELECT {ROLE_COLS} FROM roles r WHERE r.org_id = $1 AND r.id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?)
}

/// Role documents for the given ids of the org (for `included`), in id order.
pub async fn role_docs(st: &AppState, org: &str, ids: &[String]) -> Result<Vec<Value>, DdError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let rows: Vec<RoleRow> = sqlx::query_as(&format!("SELECT {ROLE_COLS} FROM roles r WHERE r.org_id = $1 AND r.id = ANY($2) ORDER BY r.id")).bind(org).bind(ids).fetch_all(&st.postgres).await?;
    Ok(rows.iter().map(|r| role_doc(r, org)).collect())
}

async fn role_or_404(st: &AppState, org: &str, id: &str) -> Result<RoleRow, DdError> {
    find_role(st, org, id).await?.ok_or_else(|| not_found(format!("Role {id} not found")))
}

async fn role_response(st: &AppState, org: &str, id: &str) -> Result<Value, DdError> {
    Ok(json!({ "data": role_doc(&role_or_404(st, org, id).await?, org) }))
}

fn is_builtin(org: &str, id: &str) -> bool {
    perms::builtin_level(org, id).is_some()
}

fn builtin_unchangeable() -> DdError {
    bad_request("Built-in roles (Admin, Standard, Read Only) can't be changed; create a custom role instead")
}

/// The permissions of `relationships.permissions`, each one we support and the caller holds.
fn requested_permissions(ctx: &AuthContext, data: &Value) -> Result<Option<Vec<&'static Permission>>, DdError> {
    let Some(ids) = relationship_ids(data, "permissions", "permissions")? else { return Ok(None) };
    ids.iter().map(|id| grantable(ctx, id)).collect::<Result<Vec<_>, _>>().map(Some)
}

fn grantable(ctx: &AuthContext, id: &str) -> Result<&'static Permission, DdError> {
    let p = perms::by_id(id).ok_or_else(|| bad_request(format!("Permission {id} does not exist or isn't supported by ObserveCraft; list them with GET /api/v2/permissions")))?;
    if p.level > ctx.access {
        return Err(DdError::new(StatusCode::FORBIDDEN, format!("Forbidden: you can't grant {}, which you don't have", p.name)));
    }
    Ok(p)
}

fn role_name(attrs: &Value) -> Result<String, DdError> {
    let name = attrs.get("name").and_then(Value::as_str).ok_or_else(|| bad_request("`name` is required"))?;
    if name.trim().is_empty() {
        return Err(bad_request("Role names cannot be only whitespace"));
    }
    check_name(name, "Role name", 100)
}

async fn name_taken(st: &AppState, org: &str, name: &str, except: Option<&str>) -> Result<bool, DdError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT id FROM roles WHERE org_id = $1 AND lower(name) = lower($2) AND id <> COALESCE($3, '')").bind(org).bind(name).bind(except).fetch_optional(&st.postgres).await?;
    Ok(row.is_some())
}

fn conflict() -> DdError {
    DdError::new(StatusCode::CONFLICT, "A role with the same name already exists")
}

async fn insert_role(st: &AppState, ctx: &AuthContext, name: &str, permissions: &[&Permission]) -> Result<String, DdError> {
    if name_taken(st, &ctx.org_id, name, None).await? {
        return Err(conflict());
    }
    let mut tx = st.postgres.begin().await?;
    let row: Option<(String,)> = sqlx::query_as("INSERT INTO roles (id, org_id, name, managed) VALUES (gen_random_uuid()::text, $1, $2, false) ON CONFLICT (org_id, name) DO NOTHING RETURNING id")
        .bind(&ctx.org_id)
        .bind(name)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((id,)) = row else { return Err(conflict()) };
    set_permissions(&mut tx, &ctx.org_id, &id, permissions).await?;
    tx.commit().await?;
    ctx.audit(st, "role.created", name, &names(permissions)).await;
    Ok(id)
}

async fn set_permissions(tx: &mut sqlx::Transaction<'_, Postgres>, org: &str, role: &str, permissions: &[&Permission]) -> Result<(), DdError> {
    sqlx::query("DELETE FROM role_permissions WHERE org_id = $1 AND role_id = $2").bind(org).bind(role).execute(&mut **tx).await?;
    let list: Vec<String> = permissions.iter().map(|p| p.name.to_string()).collect();
    sqlx::query("INSERT INTO role_permissions (org_id, role_id, permission) SELECT $1, $2, unnest($3::text[]) ON CONFLICT DO NOTHING").bind(org).bind(role).bind(&list).execute(&mut **tx).await?;
    Ok(())
}

fn names(permissions: &[&Permission]) -> String {
    permissions.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
}

// ================================================================ permissions

/// Every permission ObserveCraft supports (Datadog-compatible), with stable ids.
#[utoipa::path(get, path = "/api/v2/permissions", operation_id = "datadog_list_permissions", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_permissions(DdAuth(_auth): DdAuth) -> DocResult {
    ok(json!({ "data": perms::CATALOG.iter().map(perms::document).collect::<Vec<_>>() }))
}

// ================================================================ roles

const SORTS: &[(&str, &str)] = &[("name", "lower(x.name)"), ("modified_at", "x.created_at"), ("user_count", "x.user_count")];

/// List roles (Datadog-compatible): `filter` (name; Datadog's built-in role names find ours), `filter[id]`, paging and sort.
#[utoipa::path(get, path = "/api/v2/roles", operation_id = "datadog_list_roles", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let page = DdPage::from(&q)?;
    let filter = q.get("filter").map(|s| s.trim().to_string()).unwrap_or_default();
    let ids: Vec<String> = q.get("filter[id]").map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    let raw_sort = q.get("sort").map(String::as_str).unwrap_or("name");
    let (field, desc) = raw_sort.strip_prefix('-').map_or((raw_sort, false), |f| (f, true));
    let column = SORTS.iter().find(|(k, _)| *k == field).map(|(_, c)| *c).ok_or_else(|| bad_request(format!("unknown sort '{raw_sort}'")))?;

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!("SELECT * FROM (SELECT {ROLE_COLS}, count(*) OVER () AS filtered FROM roles r WHERE r.org_id = "));
    qb.push_bind(&ctx.org_id);
    if !filter.is_empty() {
        qb.push(" AND (r.name ILIKE ").push_bind(ilike(&filter));
        if let Some(suffix) = perms::builtin_alias(&filter) {
            qb.push(" OR r.id = ").push_bind(format!("{}-{suffix}", ctx.org_id));
        }
        qb.push(")");
    }
    if !ids.is_empty() {
        qb.push(" AND r.id = ANY(").push_bind(ids.clone()).push(")");
    }
    qb.push(format!(") x ORDER BY {column} {}, x.id LIMIT ", if desc { "DESC" } else { "ASC" })).push_bind(page.size).push(" OFFSET ").push_bind(page.offset());
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        role: RoleRow,
        filtered: i64,
    }
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&st.postgres).await?;
    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM roles WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    let filtered = rows.first().map(|r| r.filtered).unwrap_or(if filter.is_empty() && ids.is_empty() { total } else { 0 });
    let data: Vec<Value> = rows.iter().map(|r| role_doc(&r.role, &ctx.org_id)).collect();
    ok(json!({ "data": data, "meta": { "page": { "total_count": total, "total_filtered_count": filtered } } }))
}

/// One role (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/roles/{role_id}", operation_id = "datadog_get_role", params(("role_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(role_response(&st, &ctx.org_id, &id).await?)
}

/// Creates a custom role with the given permissions (Datadog-compatible).
#[utoipa::path(post, path = "/api/v2/roles", operation_id = "datadog_create_role", request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    let name = role_name(data.get("attributes").unwrap_or(&Value::Null))?;
    let permissions = requested_permissions(&ctx, data)?.unwrap_or_default();
    let id = insert_role(&st, &ctx, &name, &permissions).await?;
    ok(role_response(&st, &ctx.org_id, &id).await?)
}

/// Renames a custom role or replaces its permissions (Datadog-compatible).
#[utoipa::path(patch, path = "/api/v2/roles/{role_id}", operation_id = "datadog_update_role", params(("role_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors), (status = 422, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    if data.get("id").and_then(Value::as_str).is_some_and(|b| b != id) {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "The id attribute in the request body does not match the role_id in the URL"));
    }
    let role = role_or_404(&st, &ctx.org_id, &id).await?;
    let attrs = data.get("attributes").unwrap_or(&Value::Null);
    let name = attrs.get("name").filter(|n| !n.is_null()).map(|_| role_name(attrs)).transpose()?;
    let permissions = requested_permissions(&ctx, data)?;
    if is_builtin(&ctx.org_id, &id) {
        let renamed = name.as_ref().is_some_and(|n| *n != role.name);
        let changed = permissions.as_ref().is_some_and(|p| p.iter().map(|p| p.name).collect::<Vec<_>>() != role.permissions(&ctx.org_id).iter().map(|p| p.name).collect::<Vec<_>>());
        if renamed || changed {
            return Err(builtin_unchangeable());
        }
        return ok(json!({ "data": role_doc(&role, &ctx.org_id) }));
    }
    if let Some(name) = &name {
        if name_taken(&st, &ctx.org_id, name, Some(&id)).await? {
            return Err(conflict());
        }
    }
    let mut tx = st.postgres.begin().await?;
    if let Some(name) = &name {
        sqlx::query("UPDATE roles SET name = $3 WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(&id).bind(name).execute(&mut *tx).await?;
    }
    if let Some(p) = &permissions {
        set_permissions(&mut tx, &ctx.org_id, &id, p).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "role.updated", name.as_deref().unwrap_or(&role.name), &permissions.as_deref().map(names).unwrap_or_default()).await;
    ok(role_response(&st, &ctx.org_id, &id).await?)
}

/// Deletes a custom role (Datadog-compatible); its members lose it.
#[utoipa::path(delete, path = "/api/v2/roles/{role_id}", operation_id = "datadog_delete_role", params(("role_id" = String, Path)), responses((status = 204), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let role = role_or_404(&st, &ctx.org_id, &id).await?;
    if is_builtin(&ctx.org_id, &id) {
        return Err(bad_request("Built-in roles (Admin, Standard, Read Only) can't be deleted"));
    }
    sqlx::query("DELETE FROM roles WHERE org_id = $1 AND id = $2 AND NOT managed").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "role.deleted", &role.name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Creates a custom role with another role's permissions (Datadog-compatible).
#[utoipa::path(post, path = "/api/v2/roles/{role_id}/clone", operation_id = "datadog_clone_role", params(("role_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn clone(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let name = role_name(data(&doc)?.get("attributes").unwrap_or(&Value::Null))?;
    let source = role_or_404(&st, &ctx.org_id, &id).await?;
    let permissions = source.permissions(&ctx.org_id);
    if let Some(p) = permissions.iter().find(|p| p.level > ctx.access) {
        return Err(grantable(&ctx, &perms::id_of(p.name)).unwrap_err());
    }
    let new_id = insert_role(&st, &ctx, &name, &permissions).await?;
    ok(role_response(&st, &ctx.org_id, &new_id).await?)
}

// ================================================================ role permissions

async fn permissions_response(st: &AppState, org: &str, id: &str) -> DocResult {
    let role = role_or_404(st, org, id).await?;
    ok(json!({ "data": role.permissions(org).into_iter().map(perms::document).collect::<Vec<_>>() }))
}

/// The permissions of a role (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/roles/{role_id}/permissions", operation_id = "datadog_list_role_permissions", params(("role_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_role_permissions(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    permissions_response(&st, &ctx.org_id, &id).await
}

async fn change_permission(st: &AppState, ctx: &AuthContext, id: &str, raw: &Bytes, grant: bool) -> DocResult {
    require(ctx, Access::Admin)?;
    let doc = body(raw)?;
    let pid = relationship_id(data(&doc)?, "permissions")?;
    let role = role_or_404(st, &ctx.org_id, id).await?;
    let p = grantable(ctx, &pid)?;
    if is_builtin(&ctx.org_id, id) {
        return Err(builtin_unchangeable());
    }
    if grant {
        sqlx::query("INSERT INTO role_permissions (org_id, role_id, permission) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING").bind(&ctx.org_id).bind(id).bind(p.name).execute(&st.postgres).await?;
    } else {
        sqlx::query("DELETE FROM role_permissions WHERE org_id = $1 AND role_id = $2 AND permission = $3").bind(&ctx.org_id).bind(id).bind(p.name).execute(&st.postgres).await?;
    }
    ctx.audit(st, if grant { "role.permission_granted" } else { "role.permission_revoked" }, &role.name, p.name).await;
    permissions_response(st, &ctx.org_id, id).await
}

/// Grants a permission to a custom role (Datadog-compatible).
#[utoipa::path(post, path = "/api/v2/roles/{role_id}/permissions", operation_id = "datadog_add_permission_to_role", params(("role_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn add_permission(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    change_permission(&st, &ctx, &id, &raw, true).await
}

/// Revokes a permission from a custom role (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/roles/{role_id}/permissions", operation_id = "datadog_remove_permission_from_role", params(("role_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn remove_permission(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    change_permission(&st, &ctx, &id, &raw, false).await
}

// ================================================================ role users

async fn users_response(st: &AppState, org: &str, role: &RoleRow, q: &HashMap<String, String>) -> DocResult {
    let page = DdPage::from(q)?;
    let filter = q.get("filter").map(|s| s.trim().to_string()).unwrap_or_default();
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        user: UserRow,
        filtered: i64,
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {USER_COLS}, count(*) OVER () AS filtered FROM users u JOIN user_roles ur ON ur.user_id = u.id AND ur.role_id = $2 \
         WHERE u.org_id = $1 AND ($3 = '' OR u.name ILIKE $4 OR u.email ILIKE $4) ORDER BY lower(u.name), u.id LIMIT $5 OFFSET $6"
    ))
    .bind(org)
    .bind(&role.id)
    .bind(&filter)
    .bind(ilike(&filter))
    .bind(page.size)
    .bind(page.offset())
    .fetch_all(&st.postgres)
    .await?;
    let filtered = rows.first().map_or(0, |r| r.filtered);
    let data: Vec<Value> = rows.iter().map(|r| user_doc(&r.user, org)).collect();
    ok(json!({
        "data": data,
        "included": [role_doc(role, org)],
        "meta": { "page": { "total_count": role.user_count, "total_filtered_count": filtered } },
    }))
}

/// The users who have a role (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/roles/{role_id}/users", operation_id = "datadog_list_role_users", params(("role_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_users(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let role = role_or_404(&st, &ctx.org_id, &id).await?;
    users_response(&st, &ctx.org_id, &role, &q).await
}

async fn change_member(st: &AppState, ctx: &AuthContext, id: &str, raw: &Bytes, add: bool) -> DocResult {
    require(ctx, Access::Admin)?;
    let doc = body(raw)?;
    let user_id = relationship_id(data(&doc)?, "users")?;
    let role = role_or_404(st, &ctx.org_id, id).await?;
    let user = super::users::find_user(st, &ctx.org_id, &user_id).await?.ok_or_else(|| not_found(format!("User {user_id} not found")))?;
    if !add && user.id == ctx.user_id && role.id == format!("{}-admin", ctx.org_id) {
        return Err(bad_request("You can't remove your own admin role"));
    }
    if add {
        sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT $1::uuid, id FROM roles WHERE org_id = $2 AND id = $3 ON CONFLICT DO NOTHING")
            .bind(&user.id)
            .bind(&ctx.org_id)
            .bind(&role.id)
            .execute(&st.postgres)
            .await?;
    } else {
        sqlx::query("DELETE FROM user_roles WHERE user_id = $1::uuid AND role_id = (SELECT id FROM roles WHERE org_id = $2 AND id = $3)")
            .bind(&user.id)
            .bind(&ctx.org_id)
            .bind(&role.id)
            .execute(&st.postgres)
            .await?;
    }
    ctx.audit(st, "user.roles_changed", &user.email, &format!("{} {}", if add { "added" } else { "removed" }, role.name)).await;
    let role = role_or_404(st, &ctx.org_id, id).await?;
    users_response(st, &ctx.org_id, &role, &HashMap::from([("page[size]".to_string(), "5000".to_string())])).await
}

/// Gives a user a role (Datadog-compatible).
#[utoipa::path(post, path = "/api/v2/roles/{role_id}/users", operation_id = "datadog_add_user_to_role", params(("role_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn add_user(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    change_member(&st, &ctx, &id, &raw, true).await
}

/// Takes a role from a user (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/roles/{role_id}/users", operation_id = "datadog_remove_user_from_role", params(("role_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn remove_user(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    change_member(&st, &ctx, &id, &raw, false).await
}

#[cfg(test)]
mod tests {
    use super::super::replay::{user_with_app_key, Feature, TestServer};
    use super::super::users::tests::send;
    use super::*;
    use reqwest::Method;

    #[tokio::test]
    async fn custom_roles_read_back_their_permissions() {
        let Some(s) = TestServer::start("dd-roles").await else { return };
        let (_, list) = send(&s, &s.app_key, Method::GET, "/api/v2/permissions", None).await;
        let ids: HashMap<String, String> = list["data"].as_array().unwrap().iter().map(|p| (p["attributes"]["name"].as_str().unwrap().to_string(), p["id"].as_str().unwrap().to_string())).collect();
        assert_eq!(ids.len(), perms::CATALOG.len());
        let rel = |n: &str| json!({ "type": "permissions", "id": ids[n] });
        let body =
            json!({ "data": { "type": "roles", "attributes": { "name": "Release Managers" }, "relationships": { "permissions": { "data": [rel("monitors_write"), rel("dashboards_read")] } } } });
        let (code, created) = send(&s, &s.app_key, Method::POST, "/api/v2/roles", Some(body.clone())).await;
        assert_eq!(code, 200, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["data"]["relationships"]["permissions"]["data"], json!([rel("dashboards_read"), rel("monitors_write")]));
        let (code, _) = send(&s, &s.app_key, Method::POST, "/api/v2/roles", Some(body)).await;
        assert_eq!(code, 409);
        // The role is ours: it shows in our roles list.
        let (managed,): (bool,) = sqlx::query_as("SELECT managed FROM roles WHERE org_id = $1 AND id = $2").bind(&s.org).bind(&id).fetch_one(&s.st.postgres).await.unwrap();
        assert!(!managed);

        let patch = json!({ "data": { "type": "roles", "id": id, "attributes": { "name": "Release Captains" }, "relationships": { "permissions": { "data": [rel("slos_write")] } } } });
        let (code, updated) = send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/roles/{id}"), Some(patch)).await;
        assert_eq!(code, 200, "{updated}");
        assert_eq!(updated["data"]["attributes"]["name"], "Release Captains");
        assert_eq!(updated["data"]["relationships"]["permissions"]["data"], json!([rel("slos_write")]));
        let (code, granted) = send(&s, &s.app_key, Method::POST, &format!("/api/v2/roles/{id}/permissions"), Some(json!({ "data": rel("logs_read_data") }))).await;
        assert_eq!((code, granted["data"].as_array().unwrap().len()), (200, 2));

        let unknown = json!({ "data": { "type": "roles", "attributes": { "name": "Nope" }, "relationships": { "permissions": { "data": [{ "type": "permissions", "id": "984a2bd4-d3b4-11e8-a1ff-a7f660d43029" }] } } } });
        let (code, err) = send(&s, &s.app_key, Method::POST, "/api/v2/roles", Some(unknown)).await;
        assert_eq!(code, 400);
        assert!(err["errors"][0].as_str().unwrap().contains("984a2bd4-d3b4-11e8-a1ff-a7f660d43029"), "{err}");

        // Built-in roles: their level's permissions, found by Datadog's names, never changed.
        let (code, roles) = send(&s, &s.app_key, Method::GET, "/api/v2/roles?filter=Datadog%20Read%20Only%20Role", None).await;
        assert_eq!(code, 200);
        let ro = &roles["data"][0];
        assert_eq!(ro["id"], json!(format!("{}-readonly", s.org)), "{roles}");
        let ro_perms: Vec<&str> = ro["relationships"]["permissions"]["data"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert!(ro_perms.contains(&ids["dashboards_read"].as_str()) && !ro_perms.contains(&ids["dashboards_write"].as_str()));
        let admin = format!("{}-admin", s.org);
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/roles/{admin}"), None).await;
        assert_eq!(code, 400);
        let (code, _) = send(&s, &s.app_key, Method::POST, &format!("/api/v2/roles/{admin}/permissions"), Some(json!({ "data": rel("dashboards_read") }))).await;
        assert_eq!(code, 400);
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/roles/{admin}/users"), Some(json!({ "data": { "type": "users", "id": s.user_id } }))).await;
        assert_eq!(code, 400, "admins can't drop their own admin role");

        // Assignments, by admins only.
        let (member, member_key) = user_with_app_key(&s.st, &s.org, "standard").await;
        let (code, users) = send(&s, &s.app_key, Method::POST, &format!("/api/v2/roles/{id}/users"), Some(json!({ "data": { "type": "users", "id": member } }))).await;
        assert_eq!((code, users["data"][0]["id"].as_str()), (200, Some(member.as_str())), "{users}");
        let (code, _) = send(&s, &member_key, Method::POST, &format!("/api/v2/roles/{admin}/users"), Some(json!({ "data": { "type": "users", "id": member } }))).await;
        assert_eq!(code, 403, "a Standard user can't make themselves admin");
        let (code, _) = send(&s, &member_key, Method::POST, "/api/v2/roles", Some(json!({ "data": { "type": "roles", "attributes": { "name": "Mine" } } }))).await;
        assert_eq!(code, 403);
        let (code, _) = send(&s, &member_key, Method::GET, &format!("/api/v2/roles/{id}"), None).await;
        assert_eq!(code, 200, "everyone reads roles");

        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/roles/{id}"), None).await;
        assert_eq!(code, 204);
        let (code, _) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/roles/{id}"), None).await;
        assert_eq!(code, 404);
        let other = TestServer::start("dd-roles-other").await.unwrap();
        let (code, _) = send(&other, &other.app_key, Method::GET, &format!("/api/v2/roles/{admin}"), None).await;
        assert_eq!(code, 404, "another org's role");
    }

    #[test]
    fn nobody_grants_what_they_dont_have() {
        let mut ctx = crate::auth::AuthContext {
            user_id: String::new(),
            org_id: "acme".into(),
            account_id: String::new(),
            instance_admin: false,
            name: String::new(),
            access: Access::Standard,
            credential: crate::auth::Credential::AppKey,
            session_hash: None,
            ip: None,
        };
        assert!(grantable(&ctx, &perms::id_of("dashboards_write")).is_ok());
        assert_eq!(grantable(&ctx, &perms::id_of("user_access_manage")).unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(grantable(&ctx, "11111111-dead-beef-dead-ffffffffffff").unwrap_err().status, StatusCode::BAD_REQUEST);
        ctx.access = Access::Admin;
        assert!(grantable(&ctx, &perms::id_of("user_access_manage")).is_ok());
    }

    #[tokio::test]
    async fn replays_datadogs_roles_scenarios() {
        let Some(s) = TestServer::start("dd-roles-replay").await else { return };
        let f = Feature::load("v2/roles");
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= 20, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
