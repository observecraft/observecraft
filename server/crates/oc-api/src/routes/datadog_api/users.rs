//! Users, Datadog-compatible (`/api/v2/users`, `/api/v2/user_invitations`), on our `users` table:
//! the Terraform provider's `datadog_user` resource and `datadog_user` / `datadog_users` data sources.
//!
//! - Creating a user is our invitation: a pending membership with the given roles. Sending the
//!   invitation (`send_user_invitation`) issues the one-time invitation link; no email is sent, an
//!   admin shares the link from Organization Settings → Users.
//! - `disabled` deactivates the user (signed out everywhere, keys stop working); deleting a user
//!   deactivates it, as Datadog does. A disabled user's email answers 409 on create, which the
//!   provider follows by re-enabling it.
//! - Only admins manage users, as in our API; an admin can't disable themselves.
//!
//! Also the JSON:API helpers the other compatible identity modules share.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Postgres, QueryBuilder};
use utoipa::ToSchema;

use super::{DdAuth, DdError};
use crate::auth::{crypto, Access, AuthContext};
use crate::state::AppState;

// ================================================================ shared helpers

/// A Datadog JSON:API document (`data`, `included`, `meta`), as the compatible identity routes send and receive them.
/// (Handlers build these as JSON; the type documents them in the OpenAPI spec.)
#[derive(Serialize, ToSchema)]
#[allow(dead_code)]
pub struct DatadogIamDocument {
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

pub type DocResult = Result<(StatusCode, Json<Value>), DdError>;

pub fn ok(v: Value) -> DocResult {
    Ok((StatusCode::OK, Json(v)))
}

pub fn created(v: Value) -> DocResult {
    Ok((StatusCode::CREATED, Json(v)))
}

pub fn bad_request(m: impl Into<String>) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, m)
}

pub fn not_found(m: impl Into<String>) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, m)
}

/// Our access check, with Datadog's error shape: the same levels as our own API.
pub fn require(ctx: &AuthContext, level: Access) -> Result<(), DdError> {
    if ctx.access >= level {
        return Ok(());
    }
    Err(DdError::new(
        StatusCode::FORBIDDEN,
        match level {
            Access::Admin => "Forbidden: organization admin role required",
            _ => "Forbidden: your role is read-only",
        },
    ))
}

/// RFC 3339 from epoch milliseconds.
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.to_rfc3339()).unwrap_or_default()
}

pub fn iso_opt(ms: Option<i64>) -> Value {
    ms.map(|m| Value::String(iso(m))).unwrap_or(Value::Null)
}

/// A canonical UUID (any version), lowercased; None for anything else (answered as not found).
pub fn uuid(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let ok = s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    ok.then_some(s)
}

/// The request body as JSON.
pub fn body(bytes: &Bytes) -> Result<Value, DdError> {
    serde_json::from_slice(bytes).map_err(|e| bad_request(format!("Invalid JSON body: {e}")))
}

/// The `data` object of a request body.
pub fn data(v: &Value) -> Result<&Value, DdError> {
    v.get("data").filter(|d| d.is_object()).ok_or_else(|| bad_request("The request body needs a `data` object"))
}

/// An optional string attribute: absent → None, null → Some(None).
pub fn opt_str(attrs: &Value, key: &str) -> Result<Option<Option<String>>, DdError> {
    match attrs.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(bad_request(format!("`{key}` must be a string"))),
    }
}

/// The ids of a to-many relationship (`relationships.<name>.data[].id`), checking their type.
pub fn relationship_ids(data: &Value, name: &str, kind: &str) -> Result<Option<Vec<String>>, DdError> {
    let Some(rel) = data.get("relationships").and_then(|r| r.get(name)).and_then(|r| r.get("data")) else { return Ok(None) };
    let Some(items) = rel.as_array() else { return Err(bad_request(format!("`relationships.{name}.data` must be a list"))) };
    items.iter().map(|item| relationship_id(item, kind)).collect::<Result<Vec<_>, _>>().map(Some)
}

/// The id of a relationship item `{"type": kind, "id": …}`.
pub fn relationship_id(item: &Value, kind: &str) -> Result<String, DdError> {
    if item.get("type").and_then(Value::as_str).is_some_and(|t| t != kind) {
        return Err(bad_request(format!("expected a relationship of type `{kind}`")));
    }
    item.get("id").and_then(Value::as_str).map(str::to_string).ok_or_else(|| bad_request(format!("a `{kind}` relationship needs an `id`")))
}

/// `page[size]` (default 10, at most 5000) and `page[number]` (from 0).
pub struct DdPage {
    pub size: i64,
    pub number: i64,
}

impl DdPage {
    pub fn from(q: &HashMap<String, String>) -> Result<DdPage, DdError> {
        let int = |k: &str, default: i64| -> Result<i64, DdError> {
            match q.get(k) {
                None => Ok(default),
                Some(v) => v.trim().parse::<i64>().map_err(|_| bad_request(format!("`{k}` must be an integer"))),
            }
        };
        Ok(DdPage { size: int("page[size]", 10)?.clamp(1, 5000), number: int("page[number]", 0)?.max(0) })
    }
    pub fn offset(&self) -> i64 {
        self.size.saturating_mul(self.number)
    }
}

/// `%text%` for ILIKE, with the wildcards escaped.
pub fn ilike(s: &str) -> String {
    format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

/// A display name: trimmed, 1–max characters.
pub fn check_name(s: &str, what: &str, max: usize) -> Result<String, DdError> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(bad_request(format!("{what} must be 1–{max} characters and not only whitespace")));
    }
    Ok(t.to_string())
}

// ================================================================ user documents

#[derive(sqlx::FromRow)]
pub struct UserRow {
    pub id: String,
    pub email: String,
    pub name: String,
    pub title: Option<String>,
    pub status: String,
    pub mfa_enabled: bool,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    pub role_ids: Vec<String>,
}

/// The columns of [`UserRow`] for `users u`.
pub const USER_COLS: &str = "u.id::text AS id, u.email, u.name, u.title, u.status, u.mfa_enabled, \
    (extract(epoch FROM u.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM u.last_login_at) * 1000)::bigint AS last_login_at, \
    COALESCE((SELECT array_agg(r.role_id ORDER BY r.role_id) FROM user_roles r WHERE r.user_id = u.id), '{}') AS role_ids";

/// Datadog's user status for ours.
pub fn status(ours: &str) -> &'static str {
    match ours {
        "active" => "Active",
        "pending" => "Pending",
        _ => "Disabled",
    }
}

/// The user as a Datadog `users` document.
pub fn user_doc(u: &UserRow, org: &str) -> Value {
    let created = iso(u.created_at);
    let status = status(&u.status);
    let roles: Vec<Value> = u.role_ids.iter().map(|id| json!({ "type": "roles", "id": id })).collect();
    json!({
        "type": "users",
        "id": u.id,
        "attributes": {
            "email": u.email, "handle": u.email, "name": u.name, "title": u.title, "icon": "",
            "status": status, "disabled": status == "Disabled", "verified": status == "Active",
            "service_account": false, "mfa_enabled": u.mfa_enabled, "allowed_login_methods": [],
            "created_at": created, "modified_at": created, "last_login_time": iso_opt(u.last_login_at), "uuid": u.id,
        },
        "relationships": { "roles": { "data": roles }, "org": { "data": { "type": "orgs", "id": org } } },
    })
}

/// A user of the org, or None.
pub async fn find_user(st: &AppState, org: &str, id: &str) -> Result<Option<UserRow>, DdError> {
    let Some(id) = uuid(id) else { return Ok(None) };
    Ok(sqlx::query_as(&format!("SELECT {USER_COLS} FROM users u WHERE u.org_id = $1 AND u.id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?)
}

async fn user_response(st: &AppState, org: &str, id: &str) -> Result<Value, DdError> {
    let u = find_user(st, org, id).await?.ok_or_else(|| not_found(format!("{id} not found")))?;
    Ok(json!({ "data": user_doc(&u, org) }))
}

/// Roles of the org among `ids`; a 400 naming the first unknown one.
pub async fn check_roles(st: &AppState, org: &str, ids: &[String]) -> Result<(), DdError> {
    let found: Vec<(String,)> = sqlx::query_as("SELECT id FROM roles WHERE org_id = $1 AND id = ANY($2)").bind(org).bind(ids).fetch_all(&st.postgres).await?;
    if let Some(missing) = ids.iter().find(|id| !found.iter().any(|(f,)| f == *id)) {
        return Err(bad_request(format!("Role {missing} does not exist")));
    }
    Ok(())
}

fn valid_email(raw: &str) -> Result<String, DdError> {
    let email = raw.trim().to_lowercase();
    let valid = email.len() <= 254 && email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'));
    if !valid {
        return Err(bad_request(format!("'{raw}' is not a valid email address")));
    }
    Ok(email)
}

// ================================================================ handlers

const SORTS: &[(&str, &str)] = &[("name", "lower(u.name)"), ("email", "u.email"), ("status", "u.status"), ("modified_at", "u.created_at"), ("created_at", "u.created_at")];

/// List users (Datadog-compatible): `filter` (name or email), `filter[status]` (`Active,Pending,Disabled`), paging and sort.
#[utoipa::path(get, path = "/api/v2/users", operation_id = "datadog_list_users", responses((status = 200, body = DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let page = DdPage::from(&q)?;
    let text = q.get("filter").map(|s| s.trim().to_string()).unwrap_or_default();
    let mut statuses = vec![];
    for s in q.get("filter[status]").map(String::as_str).unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()) {
        statuses.push(match s.to_ascii_lowercase().as_str() {
            "active" => "active",
            "pending" => "pending",
            "disabled" => "deactivated",
            _ => return Err(bad_request(format!("unknown status '{s}': use Active, Pending or Disabled"))),
        });
    }
    let raw_sort = q.get("sort").map(String::as_str).unwrap_or("name");
    let (field, mut desc) = match raw_sort.strip_prefix('-') {
        Some(f) => (f, true),
        None => (raw_sort, false),
    };
    if q.get("sort_dir").is_some_and(|d| d.eq_ignore_ascii_case("desc")) {
        desc = true;
    }
    let column = SORTS.iter().find(|(k, _)| *k == field).map(|(_, c)| *c).ok_or_else(|| bad_request(format!("unknown sort '{raw_sort}'")))?;

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!("SELECT {USER_COLS}, count(*) OVER () AS filtered FROM users u WHERE u.org_id = "));
    qb.push_bind(&ctx.org_id);
    if !text.is_empty() {
        let p = ilike(&text);
        qb.push(" AND (u.name ILIKE ").push_bind(p.clone()).push(" OR u.email ILIKE ").push_bind(p).push(")");
    }
    if !statuses.is_empty() {
        qb.push(" AND u.status = ANY(").push_bind(statuses.iter().map(|s| s.to_string()).collect::<Vec<_>>()).push(")");
    }
    qb.push(format!(" ORDER BY {column} {}, u.id LIMIT ", if desc { "DESC" } else { "ASC" })).push_bind(page.size).push(" OFFSET ").push_bind(page.offset());
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        user: UserRow,
        filtered: i64,
    }
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&st.postgres).await?;
    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM users WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    let filtered = match rows.first() {
        Some(r) => r.filtered,
        None if text.is_empty() && statuses.is_empty() => total,
        None => 0,
    };
    let data: Vec<Value> = rows.iter().map(|r| user_doc(&r.user, &ctx.org_id)).collect();
    let mut role_ids: Vec<String> = rows.iter().flat_map(|r| r.user.role_ids.iter().cloned()).collect();
    role_ids.sort();
    role_ids.dedup();
    let included = super::roles::role_docs(&st, &ctx.org_id, &role_ids).await?;
    ok(json!({ "data": data, "included": included, "meta": { "page": { "total_count": total, "total_filtered_count": filtered } } }))
}

/// One user (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/users/{user_id}", operation_id = "datadog_get_user", params(("user_id" = String, Path)), responses((status = 200, body = DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(user_response(&st, &ctx.org_id, &id).await?)
}

/// Creates a pending user, our invitation (Datadog-compatible). An existing email answers 409.
#[utoipa::path(post, path = "/api/v2/users", operation_id = "datadog_create_user", request_body = DatadogIamDocument, responses((status = 201, body = DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    let attrs = data.get("attributes").cloned().unwrap_or(Value::Null);
    let email = valid_email(attrs.get("email").and_then(Value::as_str).ok_or_else(|| bad_request("`email` is required"))?)?;
    let name = match opt_str(&attrs, "name")?.flatten() {
        Some(n) => check_name(&n, "name", 120)?,
        None => email.split('@').next().unwrap_or(&email).replace(['.', '_'], " "),
    };
    let title = opt_str(&attrs, "title")?.flatten().map(|t| t.trim().chars().take(120).collect::<String>());
    let roles = relationship_ids(data, "roles", "roles")?.unwrap_or_default();
    check_roles(&st, &ctx.org_id, &roles).await?;

    let mut tx = st.postgres.begin().await?;
    let row: Option<(String,)> = sqlx::query_as(
        "INSERT INTO users (id, org_id, email, name, title, status, login_methods) VALUES (gen_random_uuid(), $1, $2, $3, $4, 'pending', '{}') \
         ON CONFLICT (org_id, email) DO NOTHING RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&email)
    .bind(&name)
    .bind(&title)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id,)) = row else { return Err(DdError::new(StatusCode::CONFLICT, format!("User with email {email} already exists"))) };
    sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT $1::uuid, id FROM roles WHERE org_id = $2 AND id = ANY($3)").bind(&id).bind(&ctx.org_id).bind(&roles).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "user.invited", &email, "Datadog-compatible API").await;
    created(user_response(&st, &ctx.org_id, &id).await?)
}

/// Updates a user's name, title or `disabled` (Datadog-compatible). Emails can't change.
#[utoipa::path(patch, path = "/api/v2/users/{user_id}", operation_id = "datadog_update_user", params(("user_id" = String, Path)), request_body = DatadogIamDocument, responses((status = 200, body = DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors), (status = 422, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    if data.get("id").and_then(Value::as_str).is_some_and(|b| !b.eq_ignore_ascii_case(&id)) {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "The user id in the request body does not match the URL"));
    }
    let user = find_user(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found(format!("{id} not found")))?;
    let attrs = data.get("attributes").cloned().unwrap_or(Value::Null);
    if let Some(Some(email)) = opt_str(&attrs, "email")? {
        if email.trim().to_lowercase() != user.email {
            return Err(bad_request("Changing a user's email isn't supported; create a user with the new email"));
        }
    }
    let name = opt_str(&attrs, "name")?.flatten().map(|n| check_name(&n, "name", 120)).transpose()?;
    let title = opt_str(&attrs, "title")?;
    let disabled = match attrs.get("disabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad_request("`disabled` must be a boolean")),
    };
    if disabled == Some(true) && user.id == ctx.user_id {
        return Err(bad_request("You can't disable yourself"));
    }
    let mut tx = st.postgres.begin().await?;
    sqlx::query("UPDATE users SET name = COALESCE($3, name) WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&user.id).bind(&name).execute(&mut *tx).await?;
    if let Some(title) = &title {
        let title = title.as_ref().map(|t| t.trim().chars().take(120).collect::<String>()).filter(|t| !t.is_empty());
        sqlx::query("UPDATE users SET title = $3 WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&user.id).bind(title).execute(&mut *tx).await?;
    }
    let action = match (disabled, user.status.as_str()) {
        (Some(true), "active" | "pending") => {
            deactivate(&mut tx, &ctx.org_id, &user.id).await?;
            Some("user.deactivated")
        }
        (Some(false), "deactivated") => {
            // Back to pending until the person has signed in once.
            sqlx::query("UPDATE users SET status = CASE WHEN last_login_at IS NULL THEN 'pending' ELSE 'active' END WHERE org_id = $1 AND id = $2::uuid")
                .bind(&ctx.org_id)
                .bind(&user.id)
                .execute(&mut *tx)
                .await?;
            Some("user.active")
        }
        _ => None,
    };
    tx.commit().await?;
    if let Some(action) = action {
        ctx.audit(&st, action, &user.email, "Datadog-compatible API").await;
    }
    ok(user_response(&st, &ctx.org_id, &user.id).await?)
}

async fn deactivate(tx: &mut sqlx::Transaction<'_, Postgres>, org: &str, user_id: &str) -> Result<(), DdError> {
    sqlx::query("UPDATE users SET status = 'deactivated' WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(user_id).execute(&mut **tx).await?;
    // Signed out everywhere; application keys stop working because the user is no longer active.
    sqlx::query("DELETE FROM sessions WHERE org_id = $1 AND user_id = $2::uuid").bind(org).bind(user_id).execute(&mut **tx).await?;
    Ok(())
}

/// Disables a user (Datadog-compatible: users are never deleted). A disabled user answers 404.
#[utoipa::path(delete, path = "/api/v2/users/{user_id}", operation_id = "datadog_disable_user", params(("user_id" = String, Path)), responses((status = 204), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn disable(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let user = find_user(&st, &ctx.org_id, &id).await?.filter(|u| u.status != "deactivated").ok_or_else(|| not_found(format!("{id} not found")))?;
    if user.id == ctx.user_id {
        return Err(bad_request("You can't disable yourself"));
    }
    let mut tx = st.postgres.begin().await?;
    deactivate(&mut tx, &ctx.org_id, &user.id).await?;
    tx.commit().await?;
    ctx.audit(&st, "user.deactivated", &user.email, "Datadog-compatible API").await;
    Ok(StatusCode::NO_CONTENT)
}

/// The permissions a user has through its roles (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/users/{user_id}/permissions", operation_id = "datadog_list_user_permissions", params(("user_id" = String, Path)), responses((status = 200, body = DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn permissions(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    let user = find_user(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found(format!("{id} not found")))?;
    let level = crate::auth::access_from_roles(&ctx.org_id, &user.role_ids);
    let data: Vec<Value> = super::permissions::granted(level).map(super::permissions::document).collect();
    ok(json!({ "data": data }))
}

// ================================================================ invitations

/// An invitation's id: the first 16 bytes of its one-time token's SHA-256, as a UUID.
pub fn invitation_id(token_hash: &[u8]) -> String {
    let hex: String = token_hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

fn invitation_hash_prefix(id: &str) -> Option<Vec<u8>> {
    let hex: String = uuid(id)?.replace('-', "");
    (0..16).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect()
}

#[derive(sqlx::FromRow)]
struct InvitationRow {
    token_hash: Vec<u8>,
    user_id: String,
    created_at: i64,
    expires_at: i64,
}

fn invitation_doc(r: &InvitationRow) -> Value {
    let id = invitation_id(&r.token_hash);
    json!({
        "type": "user_invitations",
        "id": id,
        "attributes": { "uuid": id, "invite_type": "basic_invite", "login_method": null, "created_at": iso(r.created_at), "expires_at": iso(r.expires_at) },
        "relationships": { "user": { "data": { "type": "users", "id": r.user_id } } },
    })
}

const INVITATION_COLS: &str = "token_hash, user_id::text AS user_id, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM expires_at) * 1000)::bigint AS expires_at";

/// Issues invitation links for pending users (Datadog-compatible). No email is sent: an admin
/// shares the link from Organization Settings → Users.
#[utoipa::path(post, path = "/api/v2/user_invitations", operation_id = "datadog_send_invitations", request_body = DatadogIamDocument, responses((status = 201, body = DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn send_invitations(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let items = doc.get("data").and_then(Value::as_array).ok_or_else(|| bad_request("The request body needs a `data` list"))?;
    if items.is_empty() || items.len() > 50 {
        return Err(bad_request("Send 1–50 invitations at a time"));
    }
    let mut users = vec![];
    for item in items {
        let rel = item.get("relationships").and_then(|r| r.get("user")).and_then(|u| u.get("data")).ok_or_else(|| bad_request("an invitation needs `relationships.user`"))?;
        let id = relationship_id(rel, "users")?;
        let user = find_user(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found(format!("{id} not found")))?;
        match user.status.as_str() {
            "pending" => users.push(user),
            "active" => return Err(bad_request(format!("{} has already joined; no invitation is needed", user.email))),
            _ => return Err(bad_request(format!("{} is disabled; enable the user first", user.email))),
        }
    }
    let mut data = vec![];
    for user in users {
        let url = super::super::auth::issue_link(&st, &ctx.org_id, &user.id, "invite", Some(&ctx.user_id), 72).await?;
        let token = url.rsplit('/').next().unwrap_or_default();
        let row: InvitationRow =
            sqlx::query_as(&format!("SELECT {INVITATION_COLS} FROM login_tokens WHERE org_id = $1 AND token_hash = $2")).bind(&ctx.org_id).bind(crypto::sha256(token)).fetch_one(&st.postgres).await?;
        ctx.audit(&st, "user.login_link_issued", &user.email, "invite (Datadog-compatible API)").await;
        data.push(invitation_doc(&row));
    }
    created(json!({ "data": data }))
}

/// One invitation (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/user_invitations/{user_invitation_uuid}", operation_id = "datadog_get_invitation", params(("user_invitation_uuid" = String, Path)), responses((status = 200, body = DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_invitation(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let prefix = invitation_hash_prefix(&id).ok_or_else(|| not_found(format!("Invitation {id} not found")))?;
    let row: Option<InvitationRow> = sqlx::query_as(&format!("SELECT {INVITATION_COLS} FROM login_tokens WHERE org_id = $1 AND purpose = 'invite' AND substring(token_hash FROM 1 FOR 16) = $2"))
        .bind(&ctx.org_id)
        .bind(prefix)
        .fetch_optional(&st.postgres)
        .await?;
    let row = row.ok_or_else(|| not_found(format!("Invitation {id} not found")))?;
    ok(json!({ "data": invitation_doc(&row) }))
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::replay::{user_with_app_key, Feature, TestServer};
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(uuid("6409CE70-d1d9-11ec-ad3d-da7ad0900002").as_deref(), Some("6409ce70-d1d9-11ec-ad3d-da7ad0900002"));
        assert_eq!(uuid("00000000-mismatch-body-id-ffffffffffff"), None);
        assert_eq!(uuid("invalidId"), None);
        let hash = crypto::sha256("ocl_token");
        let id = invitation_id(&hash);
        assert_eq!(invitation_hash_prefix(&id).unwrap(), hash[..16].to_vec());
        let q: HashMap<String, String> = [("page[size]".to_string(), "2".to_string()), ("page[number]".to_string(), "3".to_string())].into();
        let p = DdPage::from(&q).unwrap();
        assert_eq!((p.size, p.offset()), (2, 6));
        assert!(DdPage::from(&[("page[size]".to_string(), "x".to_string())].into()).is_err());
        assert_eq!(status("deactivated"), "Disabled");
        assert_eq!(ilike("a_b%"), "%a\\_b\\%%");
    }

    pub async fn send(s: &TestServer, key: &str, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = reqwest::Client::new().request(method, format!("{}{path}", s.base)).header("DD-APPLICATION-KEY", key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    fn email(tag: &str) -> String {
        format!("{tag}-{}@example.com", crypto::random_token("").to_lowercase().replace(['-', '_'], "")[..10])
    }

    #[tokio::test]
    async fn create_update_disable_and_recreate_a_user() {
        let Some(s) = TestServer::start("dd-users").await else { return };
        use reqwest::Method;
        let addr = email("jordan");
        let role = format!("{}-standard", s.org);
        let body =
            json!({ "data": { "type": "users", "attributes": { "email": addr.to_uppercase(), "name": "Jordan Lee" }, "relationships": { "roles": { "data": [{ "type": "roles", "id": role }] } } } });
        let (code, created) = send(&s, &s.app_key, Method::POST, "/api/v2/users", Some(body.clone())).await;
        assert_eq!(code, 201, "{created}");
        let user = &created["data"];
        let id = user["id"].as_str().unwrap().to_string();
        assert_eq!(user["attributes"]["email"], json!(addr));
        assert_eq!(user["attributes"]["status"], "Pending");
        assert_eq!(user["attributes"]["disabled"], false);
        assert_eq!(user["relationships"]["roles"]["data"], json!([{ "type": "roles", "id": role }]));
        // The same rows our API reads.
        let (status, name): (String, String) = sqlx::query_as("SELECT status, name FROM users WHERE org_id = $1 AND id = $2::uuid").bind(&s.org).bind(&id).fetch_one(&s.st.postgres).await.unwrap();
        assert_eq!((status.as_str(), name.as_str()), ("pending", "Jordan Lee"));

        let (code, again) = send(&s, &s.app_key, Method::POST, "/api/v2/users", Some(body)).await;
        assert_eq!(code, 409, "{again}");

        let (code, inv) = send(
            &s,
            &s.app_key,
            Method::POST,
            "/api/v2/user_invitations",
            Some(json!({ "data": [{ "type": "user_invitations", "relationships": { "user": { "data": { "type": "users", "id": id } } } }] })),
        )
        .await;
        assert_eq!(code, 201, "{inv}");
        let inv_id = inv["data"][0]["id"].as_str().unwrap();
        let (code, got) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/user_invitations/{inv_id}"), None).await;
        assert_eq!((code, got["data"]["relationships"]["user"]["data"]["id"].as_str()), (200, Some(id.as_str())));

        let (code, list) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/users?filter={}", addr.replace('@', "%40")), None).await;
        assert_eq!(code, 200);
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
        assert_eq!(list["meta"]["page"]["total_filtered_count"], 1);
        assert_eq!(list["meta"]["page"]["total_count"], 2);
        let (_, pending) = send(&s, &s.app_key, Method::GET, "/api/v2/users?filter[status]=Pending", None).await;
        assert_eq!(pending["data"].as_array().unwrap().len(), 1);

        let patch = json!({ "data": { "type": "users", "id": id, "attributes": { "email": addr, "name": "Jordan Q. Lee", "disabled": true } } });
        let (code, updated) = send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/users/{id}"), Some(patch)).await;
        assert_eq!(code, 200, "{updated}");
        assert_eq!((updated["data"]["attributes"]["name"].as_str(), updated["data"]["attributes"]["status"].as_str()), (Some("Jordan Q. Lee"), Some("Disabled")));
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/users/{id}"), None).await;
        assert_eq!(code, 404, "an already disabled user");

        // Re-enabling: back to pending (never signed in).
        let (code, enabled) = send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/users/{id}"), Some(json!({ "data": { "type": "users", "id": id, "attributes": { "disabled": false } } }))).await;
        assert_eq!((code, enabled["data"]["attributes"]["status"].as_str()), (200, Some("Pending")));
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/users/{id}"), None).await;
        assert_eq!(code, 204);

        let (code, err) = send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/users/{id}"), Some(json!({ "data": { "type": "users", "id": id, "attributes": { "email": email("other") } } }))).await;
        assert_eq!(code, 400, "{err}");
        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/users/{}", s.user_id), None).await;
        assert_eq!(code, 400, "admins can't disable themselves");
        let (code, err) = send(
            &s,
            &s.app_key,
            Method::POST,
            "/api/v2/users",
            Some(json!({ "data": { "type": "users", "attributes": { "email": email("x") }, "relationships": { "roles": { "data": [{ "type": "roles", "id": "other-org-admin" }] } } } })),
        )
        .await;
        assert_eq!(code, 400, "{err}");
    }

    #[tokio::test]
    async fn only_admins_manage_users_and_orgs_stay_apart() {
        let Some(s) = TestServer::start("dd-users-acl").await else { return };
        use reqwest::Method;
        let (_, standard_key) = user_with_app_key(&s.st, &s.org, "standard").await;
        let (code, err) = send(&s, &standard_key, Method::POST, "/api/v2/users", Some(json!({ "data": { "type": "users", "attributes": { "email": email("nope") } } }))).await;
        assert_eq!(code, 403, "{err}");
        assert!(err["errors"][0].as_str().unwrap().contains("admin"));
        let (code, err) = send(&s, &standard_key, Method::DELETE, &format!("/api/v2/users/{}", s.user_id), None).await;
        assert_eq!(code, 403, "{err}");
        // Reading is open to every member, as in our API.
        let (code, me) = send(&s, &standard_key, Method::GET, &format!("/api/v2/users/{}", s.user_id), None).await;
        assert_eq!(code, 200, "{me}");
        let (code, perms) = send(&s, &standard_key, Method::GET, &format!("/api/v2/users/{}/permissions", s.user_id), None).await;
        assert_eq!(code, 200);
        let names: Vec<&str> = perms["data"].as_array().unwrap().iter().map(|p| p["attributes"]["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"user_access_manage") && names.contains(&"dashboards_write"), "the admin's permissions: {names:?}");

        // Another org's user is not found.
        let other = TestServer::start("dd-users-other").await.unwrap();
        let (code, _) = send(&other, &other.app_key, Method::GET, &format!("/api/v2/users/{}", s.user_id), None).await;
        assert_eq!(code, 404);
        let (code, _) =
            send(&other, &other.app_key, Method::PATCH, &format!("/api/v2/users/{}", s.user_id), Some(json!({ "data": { "type": "users", "id": s.user_id, "attributes": { "disabled": true } } })))
                .await;
        assert_eq!(code, 404);
    }

    #[tokio::test]
    async fn replays_datadogs_users_scenarios() {
        let Some(s) = TestServer::start("dd-users-replay").await else { return };
        let f = Feature::load("v2/users");
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
        assert!(replayed >= 11, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
