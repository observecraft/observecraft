//! Teams, Datadog-compatible (`/api/v2/team`), on our `teams` / `team_members` tables: the
//! provider's `datadog_team` and `datadog_team_membership` resources and `datadog_team(s)` /
//! `datadog_team_memberships` data sources.
//!
//! - A team's Datadog id is its `teams.id` (stable across renames). Its handle can't change:
//!   incidents, on-call and log restrictions refer to teams by handle.
//! - Team links (`datadog_team_link`) aren't part of our model: a team has none, and creating one
//!   is rejected. So are the display fields avatar, banner and visible/hidden modules.
//! - Membership roles: `admin`, or none (a member).
//! - As in our API, Standard users create and edit teams. Membership can grant access (log
//!   restriction queries can be scoped to teams), so adding or removing members and deleting a
//!   team are for admins.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder};

use super::users::{bad_request, body, check_name, created, data, find_user, ilike, iso, not_found, ok, opt_str, relationship_id, relationship_ids, require, uuid, DdPage, DocResult};
use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct TeamRow {
    id: String,
    handle: String,
    name: String,
    description: String,
    created_at: i64,
    user_count: i64,
}

const TEAM_COLS: &str = "t.id::text AS id, t.handle, t.name, t.description, (extract(epoch FROM t.created_at) * 1000)::bigint AS created_at, \
    (SELECT count(*) FROM team_members m WHERE m.org_id = t.org_id AND m.handle = t.handle) AS user_count";

/// A brief summary of the description: its first line, at most 120 characters.
fn summary(description: &str) -> Option<String> {
    let line = description.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(120).collect())
}

fn team_doc(t: &TeamRow) -> Value {
    let created = iso(t.created_at);
    let description = (!t.description.is_empty()).then_some(t.description.as_str());
    json!({
        "type": "team",
        "id": t.id,
        "attributes": {
            "name": t.name, "handle": t.handle, "description": description, "summary": summary(&t.description),
            "avatar": null, "banner": null, "visible_modules": [], "hidden_modules": [], "is_managed": false,
            "created_at": created, "modified_at": created, "user_count": t.user_count, "link_count": 0,
        },
        "relationships": {
            "team_links": { "data": [], "links": { "related": format!("/api/v2/team/{}/links", t.id) } },
            "user_team_permissions": { "links": { "related": format!("/api/v2/team/{}/permission-settings", t.id) } },
        },
    })
}

async fn find_team(st: &AppState, org: &str, id: &str) -> Result<TeamRow, DdError> {
    let not_found = || not_found(format!("Team {id} not found"));
    let id = uuid(id).ok_or_else(not_found)?;
    let row: Option<TeamRow> = sqlx::query_as(&format!("SELECT {TEAM_COLS} FROM teams t WHERE t.org_id = $1 AND t.id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?;
    row.ok_or_else(not_found)
}

/// Rejects the Datadog team fields we don't keep, unless empty.
fn reject_display_fields(attrs: &Value) -> Result<(), DdError> {
    for key in ["avatar", "banner", "visible_modules", "hidden_modules"] {
        match attrs.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Array(a)) if a.is_empty() => {}
            Some(_) => return Err(bad_request(format!("`{key}` isn't supported by ObserveCraft teams"))),
        }
    }
    Ok(())
}

fn valid_handle(raw: &str) -> Result<String, DdError> {
    let handle = raw.trim().to_lowercase();
    if handle.is_empty() || handle.len() > 64 || !handle.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return Err(bad_request("handle must be 1–64 characters: letters, digits, '-', '_' or '.'"));
    }
    Ok(handle)
}

// ================================================================ teams

/// List teams (Datadog-compatible): `filter[keyword]`, `filter[me]`, paging and sort.
#[utoipa::path(get, path = "/api/v2/team", operation_id = "datadog_list_teams", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let page = DdPage::from(&q)?;
    let keyword = q.get("filter[keyword]").map(|s| s.trim().to_string()).unwrap_or_default();
    let me = q.get("filter[me]").is_some_and(|v| v == "true");
    let order = match q.get("sort").map(String::as_str).unwrap_or("name") {
        "name" => "lower(x.name), x.handle",
        "-name" => "lower(x.name) DESC, x.handle",
        "user_count" => "x.user_count, x.handle",
        "-user_count" => "x.user_count DESC, x.handle",
        other => return Err(bad_request(format!("unknown sort '{other}'"))),
    };
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!("SELECT * FROM (SELECT {TEAM_COLS}, count(*) OVER () AS total FROM teams t WHERE t.org_id = "));
    qb.push_bind(&ctx.org_id);
    if !keyword.is_empty() {
        let p = ilike(&keyword);
        qb.push(" AND (t.name ILIKE ").push_bind(p.clone()).push(" OR t.handle ILIKE ").push_bind(p.clone()).push(" OR t.description ILIKE ").push_bind(p).push(")");
    }
    if me {
        qb.push(" AND t.handle IN (SELECT handle FROM team_members WHERE org_id = t.org_id AND user_id = ").push_bind(&ctx.user_id).push("::uuid)");
    }
    qb.push(format!(") x ORDER BY {order} LIMIT ")).push_bind(page.size).push(" OFFSET ").push_bind(page.offset());
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        team: TeamRow,
        total: i64,
    }
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&st.postgres).await?;
    let total = match rows.first() {
        Some(r) => r.total,
        None => {
            let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM teams WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
            if keyword.is_empty() && !me {
                n
            } else {
                0
            }
        }
    };
    let last = ((total - 1).max(0)) / page.size;
    let link = |n: i64| format!("/api/v2/team?page[number]={n}&page[size]={}", page.size);
    ok(json!({
        "data": rows.iter().map(|r| team_doc(&r.team)).collect::<Vec<_>>(),
        "meta": { "pagination": {
            "type": "number_size", "number": page.number, "size": page.size, "total": total,
            "first_number": 0, "prev_number": (page.number - 1).max(0), "next_number": page.number + 1, "last_number": last,
            "offset": page.offset(), "limit": page.size, "first_offset": 0, "prev_offset": (page.offset() - page.size).max(0),
            "next_offset": page.offset() + page.size, "last_offset": last * page.size,
        } },
        "links": { "self": link(page.number), "first": link(0), "last": link(last), "next": link(page.number + 1), "prev": (page.number > 0).then(|| link(page.number - 1)) },
    }))
}

/// One team (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/team/{team_id}", operation_id = "datadog_get_team", params(("team_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": team_doc(&find_team(&st, &ctx.org_id, &id).await?) }))
}

/// Creates a team (Datadog-compatible). Initial members (`relationships.users`) need the admin role.
#[utoipa::path(post, path = "/api/v2/team", operation_id = "datadog_create_team", request_body = super::users::DatadogIamDocument, responses((status = 201, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    let attrs = data.get("attributes").cloned().unwrap_or(Value::Null);
    reject_display_fields(&attrs)?;
    let handle = valid_handle(attrs.get("handle").and_then(Value::as_str).ok_or_else(|| bad_request("`handle` is required"))?)?;
    let name = check_name(attrs.get("name").and_then(Value::as_str).ok_or_else(|| bad_request("`name` is required"))?, "name", 120)?;
    let description = opt_str(&attrs, "description")?.flatten().unwrap_or_default();
    let members = relationship_ids(data, "users", "users")?.unwrap_or_default();
    if !members.is_empty() {
        require(&ctx, Access::Admin)?;
    }
    let mut users = vec![];
    for id in &members {
        users.push(find_user(&st, &ctx.org_id, id).await?.ok_or_else(|| not_found(format!("User {id} not found")))?.id);
    }
    let mut tx = st.postgres.begin().await?;
    let row: Option<(String,)> = sqlx::query_as("INSERT INTO teams (org_id, handle, name, description, created_via) VALUES ($1, $2, $3, $4, 'api') ON CONFLICT DO NOTHING RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&handle)
        .bind(&name)
        .bind(description.trim())
        .fetch_optional(&mut *tx)
        .await?;
    let Some((id,)) = row else { return Err(DdError::new(StatusCode::CONFLICT, format!("A team with handle '{handle}' already exists"))) };
    for user in &users {
        sqlx::query("INSERT INTO team_members (org_id, handle, user_id) VALUES ($1, $2, $3::uuid) ON CONFLICT DO NOTHING").bind(&ctx.org_id).bind(&handle).bind(user).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "team.created", &handle, "Datadog-compatible API").await;
    created(json!({ "data": team_doc(&find_team(&st, &ctx.org_id, &id).await?) }))
}

/// Renames a team or changes its description (Datadog-compatible). The handle can't change.
#[utoipa::path(patch, path = "/api/v2/team/{team_id}", operation_id = "datadog_update_team", params(("team_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    let team = find_team(&st, &ctx.org_id, &id).await?;
    let attrs = data.get("attributes").cloned().unwrap_or(Value::Null);
    reject_display_fields(&attrs)?;
    if let Some(handle) = attrs.get("handle").and_then(Value::as_str) {
        if valid_handle(handle)? != team.handle {
            return Err(bad_request(format!("A team's handle can't change (incidents, on-call and log restrictions refer to '{}'); create a new team instead", team.handle)));
        }
    }
    let name = attrs.get("name").and_then(Value::as_str).map(|n| check_name(n, "name", 120)).transpose()?;
    let description = opt_str(&attrs, "description")?.map(|d| d.unwrap_or_default().trim().to_string());
    if data.get("relationships").and_then(|r| r.get("team_links")).and_then(|l| l.get("data")).and_then(Value::as_array).is_some_and(|l| !l.is_empty()) {
        return Err(bad_request("Team links aren't supported by ObserveCraft"));
    }
    sqlx::query("UPDATE teams SET name = COALESCE($3, name), description = COALESCE($4, description) WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&team.id)
        .bind(&name)
        .bind(&description)
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "team.updated", &team.handle, "Datadog-compatible API").await;
    ok(json!({ "data": team_doc(&find_team(&st, &ctx.org_id, &id).await?) }))
}

/// Deletes a team and its memberships (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/team/{team_id}", operation_id = "datadog_delete_team", params(("team_id" = String, Path)), responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let team = find_team(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM teams WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&team.id).execute(&st.postgres).await?;
    ctx.audit(&st, "team.deleted", &team.handle, "Datadog-compatible API").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ links

/// A team's links (Datadog-compatible): always none.
#[utoipa::path(get, path = "/api/v2/team/{team_id}/links", operation_id = "datadog_list_team_links", params(("team_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn links(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    find_team(&st, &ctx.org_id, &id).await?;
    ok(json!({ "data": [] }))
}

/// Creating or changing a team link (Datadog-compatible): not supported.
#[utoipa::path(post, path = "/api/v2/team/{team_id}/links", operation_id = "datadog_create_team_link", params(("team_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_link(DdAuth(_auth): DdAuth, Path(_id): Path<String>) -> Result<StatusCode, DdError> {
    Err(bad_request("Team links aren't supported by ObserveCraft: a team has no links"))
}

/// Changing a team link (Datadog-compatible): not supported.
#[utoipa::path(patch, path = "/api/v2/team/{team_id}/links/{link_id}", operation_id = "datadog_update_team_link", params(("team_id" = String, Path), ("link_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_link(DdAuth(_auth): DdAuth, Path(_ids): Path<(String, String)>) -> Result<StatusCode, DdError> {
    Err(bad_request("Team links aren't supported by ObserveCraft: a team has no links"))
}

/// One team link (Datadog-compatible): teams have none.
#[utoipa::path(get, path = "/api/v2/team/{team_id}/links/{link_id}", operation_id = "datadog_get_team_link", params(("team_id" = String, Path), ("link_id" = String, Path)), responses((status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn link(DdAuth(_auth): DdAuth, Path(ids): Path<(String, String)>) -> Result<StatusCode, DdError> {
    Err(not_found(format!("Team link {} not found", ids.1)))
}

// ================================================================ memberships

#[derive(sqlx::FromRow)]
struct MemberRow {
    team_id: String,
    role: String,
    #[sqlx(flatten)]
    user: super::users::UserRow,
}

fn membership_doc(m: &MemberRow) -> Value {
    json!({
        "type": "team_memberships",
        "id": format!("TeamMembership-{}-{}", m.team_id, m.user.id),
        "attributes": { "role": (m.role == "admin").then_some("admin"), "provisioned_by": null, "provisioned_by_id": null },
        "relationships": {
            "team": { "data": { "type": "team", "id": m.team_id } },
            "user": { "data": { "type": "users", "id": m.user.id } },
        },
    })
}

fn member_select() -> String {
    format!("SELECT t.id::text AS team_id, m.role, {} FROM team_members m JOIN teams t ON t.org_id = m.org_id AND t.handle = m.handle JOIN users u ON u.id = m.user_id", super::users::USER_COLS)
}

fn memberships_page(rows: &[MemberRow], org: &str, total: i64, page: &DdPage, path: &str) -> Value {
    let link = |offset: i64| format!("{path}?page[offset]={offset}&page[limit]={}", page.size);
    json!({
        "data": rows.iter().map(membership_doc).collect::<Vec<_>>(),
        "included": rows.iter().map(|m| super::users::user_doc(&m.user, org)).collect::<Vec<_>>(),
        "meta": { "pagination": {
            "type": "offset_limit", "offset": page.offset(), "limit": page.size, "total": total,
            "first_offset": 0, "prev_offset": (page.offset() - page.size).max(0), "next_offset": page.offset() + page.size, "last_offset": ((total - 1).max(0) / page.size) * page.size,
        } },
        "links": { "self": path, "first": link(0), "next": link(page.offset() + page.size), "prev": null, "last": null },
    })
}

/// A team's members (Datadog-compatible): `filter[keyword]`, paging and sort.
#[utoipa::path(get, path = "/api/v2/team/{team_id}/memberships", operation_id = "datadog_list_team_memberships", params(("team_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn memberships(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let page = DdPage::from(&q)?;
    let team = find_team(&st, &ctx.org_id, &id).await?;
    let keyword = q.get("filter[keyword]").map(|s| s.trim().to_string()).unwrap_or_default();
    let order = match q.get("sort").map(String::as_str).unwrap_or("name") {
        "name" | "manager_name" | "-manager_name" => "lower(u.name), u.id",
        "-name" => "lower(u.name) DESC, u.id",
        "email" => "u.email",
        "-email" => "u.email DESC",
        "role" => "m.role, lower(u.name)",
        "-role" => "m.role DESC, lower(u.name)",
        other => return Err(bad_request(format!("unknown sort '{other}'"))),
    };
    let rows: Vec<MemberRow> =
        sqlx::query_as(&format!("{} WHERE m.org_id = $1 AND m.handle = $2 AND ($3 = '' OR u.name ILIKE $4 OR u.email ILIKE $4) ORDER BY {order} LIMIT $5 OFFSET $6", member_select()))
            .bind(&ctx.org_id)
            .bind(&team.handle)
            .bind(&keyword)
            .bind(ilike(&keyword))
            .bind(page.size)
            .bind(page.offset())
            .fetch_all(&st.postgres)
            .await?;
    ok(memberships_page(&rows, &ctx.org_id, team.user_count, &page, &format!("/api/v2/team/{}/memberships", team.id)))
}

/// The teams a user belongs to (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/users/{user_uuid}/memberships", operation_id = "datadog_get_user_memberships", params(("user_uuid" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn user_memberships(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    let user = find_user(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found(format!("User {id} not found")))?;
    let rows: Vec<MemberRow> =
        sqlx::query_as(&format!("{} WHERE m.org_id = $1 AND m.user_id = $2::uuid ORDER BY lower(t.name)", member_select())).bind(&ctx.org_id).bind(&user.id).fetch_all(&st.postgres).await?;
    ok(json!({ "data": rows.iter().map(membership_doc).collect::<Vec<_>>() }))
}

/// `admin`, or a plain member when the role is null or absent (the provider leaves it out to drop
/// the admin role); anything else is rejected.
fn member_role(attrs: Option<&Value>) -> Result<&'static str, DdError> {
    match attrs.and_then(|a| a.get("role")) {
        None | Some(Value::Null) => Ok("member"),
        Some(Value::String(r)) if r == "admin" => Ok("admin"),
        Some(other) => Err(bad_request(format!("Invalid team role {other}: use \"admin\" or null"))),
    }
}

async fn membership(st: &AppState, ctx: &AuthContext, team_handle: &str, user_id: &str) -> Result<Option<MemberRow>, DdError> {
    Ok(sqlx::query_as(&format!("{} WHERE m.org_id = $1 AND m.handle = $2 AND m.user_id = $3::uuid", member_select()))
        .bind(&ctx.org_id)
        .bind(team_handle)
        .bind(user_id)
        .fetch_optional(&st.postgres)
        .await?)
}

fn membership_response(m: &MemberRow, org: &str) -> Value {
    json!({ "data": membership_doc(m), "included": [super::users::user_doc(&m.user, org)] })
}

/// Adds a user to a team (Datadog-compatible).
#[utoipa::path(post, path = "/api/v2/team/{team_id}/memberships", operation_id = "datadog_create_team_membership", params(("team_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn add_member(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    let rel = data.get("relationships").and_then(|r| r.get("user")).and_then(|u| u.get("data")).ok_or_else(|| bad_request("a membership needs `relationships.user`"))?;
    let user_id = relationship_id(rel, "users")?;
    let role = member_role(data.get("attributes"))?;
    let team = find_team(&st, &ctx.org_id, &id).await?;
    let user = find_user(&st, &ctx.org_id, &user_id).await?.ok_or_else(|| not_found(format!("User {user_id} not found")))?;
    let done = sqlx::query("INSERT INTO team_members (org_id, handle, user_id, role) VALUES ($1, $2, $3::uuid, $4) ON CONFLICT DO NOTHING")
        .bind(&ctx.org_id)
        .bind(&team.handle)
        .bind(&user.id)
        .bind(role)
        .execute(&st.postgres)
        .await?;
    if done.rows_affected() == 0 {
        return Err(DdError::new(StatusCode::CONFLICT, format!("{} is already a member of {}", user.email, team.handle)));
    }
    ctx.audit(&st, "team.member_added", &team.handle, &user.email).await;
    let m = membership(&st, &ctx, &team.handle, &user.id).await?.ok_or_else(|| not_found("Membership not found"))?;
    ok(membership_response(&m, &ctx.org_id))
}

/// Changes a member's team role (Datadog-compatible).
#[utoipa::path(patch, path = "/api/v2/team/{team_id}/memberships/{user_id}", operation_id = "datadog_update_team_membership", params(("team_id" = String, Path), ("user_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_member(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path((id, user_id)): Path<(String, String)>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let role = member_role(data(&doc)?.get("attributes"))?;
    let team = find_team(&st, &ctx.org_id, &id).await?;
    let not_member = || not_found(format!("User {user_id} is not a member of team {id}"));
    let uid = uuid(&user_id).ok_or_else(not_member)?;
    let m = membership(&st, &ctx, &team.handle, &uid).await?.ok_or_else(not_member)?;
    if role != m.role {
        sqlx::query("UPDATE team_members SET role = $4 WHERE org_id = $1 AND handle = $2 AND user_id = $3::uuid")
            .bind(&ctx.org_id)
            .bind(&team.handle)
            .bind(&uid)
            .bind(role)
            .execute(&st.postgres)
            .await?;
        ctx.audit(&st, "team.member_role_changed", &team.handle, &format!("{}: {role}", m.user.email)).await;
    }
    let m = membership(&st, &ctx, &team.handle, &uid).await?.ok_or_else(not_member)?;
    ok(membership_response(&m, &ctx.org_id))
}

/// Removes a user from a team (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/team/{team_id}/memberships/{user_id}", operation_id = "datadog_delete_team_membership", params(("team_id" = String, Path), ("user_id" = String, Path)), responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn remove_member(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path((id, user_id)): Path<(String, String)>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let team = find_team(&st, &ctx.org_id, &id).await?;
    let not_member = || not_found(format!("User {user_id} is not a member of team {id}"));
    let uid = uuid(&user_id).ok_or_else(not_member)?;
    let done = sqlx::query("DELETE FROM team_members WHERE org_id = $1 AND handle = $2 AND user_id = $3::uuid").bind(&ctx.org_id).bind(&team.handle).bind(&uid).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_member());
    }
    ctx.audit(&st, "team.member_removed", &team.handle, &user_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::super::replay::{user_with_app_key, Feature, TestServer};
    use super::super::users::tests::send;
    use super::*;
    use reqwest::Method;

    #[test]
    fn summaries_and_roles() {
        assert_eq!(summary("\n  Owns checkout.\nMore text."), Some("Owns checkout.".into()));
        assert_eq!(summary(""), None);
        assert_eq!(member_role(Some(&json!({ "role": "admin" }))).unwrap(), "admin");
        assert_eq!(member_role(Some(&json!({ "role": null }))).unwrap(), "member");
        assert_eq!(member_role(Some(&json!({}))).unwrap(), "member");
        assert_eq!(member_role(None).unwrap(), "member");
        assert!(member_role(Some(&json!({ "role": "owner" }))).is_err());
        assert!(reject_display_fields(&json!({ "avatar": null, "hidden_modules": [] })).is_ok());
        assert!(reject_display_fields(&json!({ "banner": 7 })).is_err());
    }

    #[tokio::test]
    async fn teams_and_memberships() {
        let Some(s) = TestServer::start("dd-teams").await else { return };
        let body = json!({ "data": { "type": "team", "attributes": { "handle": "checkout-squad", "name": "Checkout Squad", "description": "Owns the storefront checkout.\n\nOn call weekly." } } });
        let (code, created) = send(&s, &s.app_key, Method::POST, "/api/v2/team", Some(body.clone())).await;
        assert_eq!(code, 201, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["data"]["attributes"]["summary"], "Owns the storefront checkout.");
        assert_eq!(created["data"]["attributes"]["user_count"], 0);
        let (code, _) = send(&s, &s.app_key, Method::POST, "/api/v2/team", Some(body)).await;
        assert_eq!(code, 409);
        // Our team: same row our API lists.
        let (via,): (String,) = sqlx::query_as("SELECT created_via FROM teams WHERE org_id = $1 AND handle = 'checkout-squad'").bind(&s.org).fetch_one(&s.st.postgres).await.unwrap();
        assert_eq!(via, "api");

        let (member, member_key) = user_with_app_key(&s.st, &s.org, "standard").await;
        let (code, _) = send(
            &s,
            &member_key,
            Method::POST,
            &format!("/api/v2/team/{id}/memberships"),
            Some(json!({ "data": { "type": "team_memberships", "relationships": { "user": { "data": { "type": "users", "id": member } } } } })),
        )
        .await;
        assert_eq!(code, 403, "joining a team can grant log access: admins only");
        let (code, m) = send(
            &s,
            &s.app_key,
            Method::POST,
            &format!("/api/v2/team/{id}/memberships"),
            Some(json!({ "data": { "type": "team_memberships", "relationships": { "user": { "data": { "type": "users", "id": member } } } } })),
        )
        .await;
        assert_eq!(code, 200, "{m}");
        assert_eq!(m["data"]["attributes"]["role"], Value::Null);
        assert_eq!(m["data"]["relationships"]["user"]["data"]["id"], json!(member));
        let (code, m) =
            send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/team/{id}/memberships/{member}"), Some(json!({ "data": { "type": "team_memberships", "attributes": { "role": "admin" } } }))).await;
        assert_eq!((code, m["data"]["attributes"]["role"].as_str()), (200, Some("admin")));
        let (code, m) = send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/team/{id}/memberships/{member}"), Some(json!({ "data": { "type": "team_memberships", "attributes": {} } }))).await;
        assert_eq!((code, &m["data"]["attributes"]["role"]), (200, &Value::Null), "no role: a plain member again");
        let (code, _) =
            send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/team/{id}/memberships/{member}"), Some(json!({ "data": { "type": "team_memberships", "attributes": { "role": "admin" } } }))).await;
        assert_eq!(code, 200);
        let (_, list) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/team/{id}/memberships"), None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
        let (_, team) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/team/{id}"), None).await;
        assert_eq!(team["data"]["attributes"]["user_count"], 1);
        let (_, mine) = send(&s, &member_key, Method::GET, "/api/v2/team?filter[me]=true", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);

        // Standard users edit teams as in our API, but the handle stays.
        let (code, updated) = send(
            &s,
            &member_key,
            Method::PATCH,
            &format!("/api/v2/team/{id}"),
            Some(json!({ "data": { "type": "team", "attributes": { "handle": "checkout-squad", "name": "Checkout", "description": "" } } })),
        )
        .await;
        assert_eq!(code, 200, "{updated}");
        assert_eq!((updated["data"]["attributes"]["name"].as_str(), updated["data"]["attributes"]["description"].is_null()), (Some("Checkout"), true));
        let (code, _) =
            send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/team/{id}"), Some(json!({ "data": { "type": "team", "attributes": { "handle": "payments", "name": "Checkout" } } }))).await;
        assert_eq!(code, 400);
        let (code, _) = send(
            &s,
            &s.app_key,
            Method::POST,
            &format!("/api/v2/team/{id}/links"),
            Some(json!({ "data": { "type": "team_links", "attributes": { "label": "Runbook", "url": "https://example.com" } } })),
        )
        .await;
        assert_eq!(code, 400, "team links aren't supported");

        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/team/{id}/memberships/{member}"), None).await;
        assert_eq!(code, 204);
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/team/{id}/memberships/{member}"), None).await;
        assert_eq!(code, 404);
        let other = TestServer::start("dd-teams-other").await.unwrap();
        let (code, _) = send(&other, &other.app_key, Method::GET, &format!("/api/v2/team/{id}"), None).await;
        assert_eq!(code, 404, "another org's team");
        let (code, _) = send(&s, &member_key, Method::DELETE, &format!("/api/v2/team/{id}"), None).await;
        assert_eq!(code, 403);
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/team/{id}"), None).await;
        assert_eq!(code, 204);
        let (code, _) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/team/{id}"), None).await;
        assert_eq!(code, 404);
    }

    #[tokio::test]
    async fn replays_datadogs_teams_scenarios() {
        let Some(s) = TestServer::start("dd-teams-replay").await else { return };
        let f = Feature::load("v2/teams");
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            // Datadog gives every team a random banner and records who added a member; our teams have
            // no banner and memberships don't record who added them, so both are null.
            problems.extend(p.into_iter().filter(|m| !m.contains("`data.attributes.banner` is null") && !m.contains("`data.attributes.provisioned_by_id` is null")));
            replayed += 1;
        }
        assert!(replayed >= 13, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
