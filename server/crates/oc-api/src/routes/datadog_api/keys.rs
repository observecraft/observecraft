//! API and application keys, Datadog-compatible (`/api/v2/api_keys`,
//! `/api/v2/current_user/application_keys`, `/api/v2/application_keys`): the provider's
//! `datadog_api_key` and `datadog_application_key` resources and the `datadog_api_key` data source.
//!
//! - A Datadog API key is one of our ingest keys (`ocik_…`), an application key one of ours
//!   (`ocak_…`). Secrets are shown once, on create, and stored hashed: later reads have no `key`
//!   (and an empty `last4`), so the provider keeps the value it got on create, and the deprecated
//!   `datadog_api_key` data source, which needs the key, reports that it can't read it.
//! - Deleting a key revokes it.
//! - Ingest keys are managed by admins (as in our API); every member can list them. Application keys
//!   belong to the calling user, who may create their own through the API too (Terraform authenticates
//!   with one); the organization-wide `/application_keys` routes are for admins.
//! - Scoped application keys and remote-configuration API keys are rejected: our keys act with their
//!   user's role, and there's no remote configuration.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::users::{bad_request, body, check_name, created, data, ilike, iso, iso_opt, not_found, ok, require, uuid, DdPage, DocResult};
use super::{DdAuth, DdError};
use crate::auth::{crypto, Access, AuthContext};
use crate::state::AppState;

/// Active application keys a user may hold at once (as in our API).
const MAX_APP_KEYS: i64 = 25;

fn key_name(attrs: &Value) -> Result<String, DdError> {
    check_name(attrs.get("name").and_then(Value::as_str).ok_or_else(|| bad_request("`name` is required"))?, "name", 100)
}

fn mismatched_id(data: &Value, id: &str) -> Result<(), DdError> {
    if data.get("id").and_then(Value::as_str).is_some_and(|b| !b.eq_ignore_ascii_case(id)) {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "The id in the request body does not match the URL"));
    }
    Ok(())
}

fn last4(key: &str) -> String {
    key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect()
}

/// `(sort column, descending)` for the `sort` parameter of key lists.
fn sort_order(q: &HashMap<String, String>, columns: &[(&str, &'static str)]) -> Result<String, DdError> {
    let raw = q.get("sort").map(String::as_str).unwrap_or("name");
    let (field, desc) = raw.strip_prefix('-').map_or((raw, false), |f| (f, true));
    let column = columns.iter().find(|(k, _)| *k == field).map(|(_, c)| *c).ok_or_else(|| bad_request(format!("unknown sort '{raw}'")))?;
    Ok(format!("{column} {}, k.id", if desc { "DESC" } else { "ASC" }))
}

// ================================================================ API keys = ingest keys

#[derive(sqlx::FromRow)]
struct ApiKeyRow {
    id: String,
    name: String,
    created_by: Option<String>,
    created_at: i64,
    last_used_at: Option<i64>,
}

const API_KEY_COLS: &str = "k.id::text AS id, k.name, k.created_by::text AS created_by, (extract(epoch FROM k.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM k.last_used_at) * 1000)::bigint AS last_used_at";

fn api_key_doc(k: &ApiKeyRow, secret: Option<&str>) -> Value {
    let created = iso(k.created_at);
    let recent = k.last_used_at.is_some_and(|t| crate::routes::metrics::now_ms() - t < 86_400_000);
    let mut attrs = json!({
        "name": k.name, "created_at": created, "modified_at": created, "last4": secret.map(last4).unwrap_or_default(),
        "category": "default", "remote_config_read_enabled": false, "private_action_runner_enroll_enabled": false,
        "date_last_used": iso_opt(k.last_used_at), "used_in_last_24_hours": recent,
        "last_used_date": { "timestamp": iso_opt(k.last_used_at), "description": "When the key last authenticated a request, or null" },
    });
    if let Some(secret) = secret {
        attrs["key"] = json!(secret);
    }
    let mut rels = json!({ "leak_information": { "data": null } });
    if let Some(user) = &k.created_by {
        rels["created_by"] = json!({ "data": { "type": "users", "id": user } });
        rels["modified_by"] = json!({ "data": { "type": "users", "id": user } });
    }
    json!({ "type": "api_keys", "id": k.id, "attributes": attrs, "relationships": rels })
}

async fn find_api_key(st: &AppState, org: &str, id: &str) -> Result<ApiKeyRow, DdError> {
    let not_found = || not_found("API key not found");
    let id = uuid(id).ok_or_else(not_found)?;
    let row: Option<ApiKeyRow> =
        sqlx::query_as(&format!("SELECT {API_KEY_COLS} FROM ingest_keys k WHERE k.org_id = $1 AND k.id = $2::uuid AND k.revoked_at IS NULL")).bind(org).bind(&id).fetch_optional(&st.postgres).await?;
    row.ok_or_else(not_found)
}

/// Rejects what our ingest keys can't do.
fn check_api_key_attrs(attrs: &Value) -> Result<(), DdError> {
    if attrs.get("remote_config_read_enabled").and_then(Value::as_bool) == Some(true) {
        return Err(bad_request("Remote configuration isn't supported by ObserveCraft: remote_config_read_enabled must be false"));
    }
    if let Some(category) = attrs.get("category").and_then(Value::as_str).filter(|c| *c != "default") {
        return Err(bad_request(format!("API key category '{category}' isn't supported: use 'default'")));
    }
    Ok(())
}

/// The organization's API keys (Datadog-compatible): our active ingest keys, without their secrets.
#[utoipa::path(get, path = "/api/v2/api_keys", operation_id = "datadog_list_api_keys", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_api_keys(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    let page = DdPage::from(&q)?;
    let filter = q.get("filter").map(|s| s.trim().to_string()).unwrap_or_default();
    let order = sort_order(&q, &[("name", "lower(k.name)"), ("created_at", "k.created_at"), ("modified_at", "k.created_at"), ("last4", "k.created_at")])?;
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        key: ApiKeyRow,
        filtered: i64,
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {API_KEY_COLS}, count(*) OVER () AS filtered FROM ingest_keys k WHERE k.org_id = $1 AND k.revoked_at IS NULL AND ($2 = '' OR k.name ILIKE $3) ORDER BY {order} LIMIT $4 OFFSET $5"
    ))
    .bind(&ctx.org_id)
    .bind(&filter)
    .bind(ilike(&filter))
    .bind(page.size)
    .bind(page.offset())
    .fetch_all(&st.postgres)
    .await?;
    let filtered = rows.first().map_or(0, |r| r.filtered);
    ok(json!({
        "data": rows.iter().map(|r| api_key_doc(&r.key, None)).collect::<Vec<_>>(),
        "meta": { "page": { "total_filtered_count": filtered }, "max_allowed": super::super::ingest_keys::MAX_KEYS },
    }))
}

/// One API key (Datadog-compatible), without its secret.
#[utoipa::path(get, path = "/api/v2/api_keys/{api_key_id}", operation_id = "datadog_get_api_key", params(("api_key_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_api_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": api_key_doc(&find_api_key(&st, &ctx.org_id, &id).await?, None) }))
}

/// Creates an ingest key (Datadog-compatible); the secret (`ocik_…`) is in this answer only.
#[utoipa::path(post, path = "/api/v2/api_keys", operation_id = "datadog_create_api_key", request_body = super::users::DatadogIamDocument, responses((status = 201, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_api_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let attrs = data(&doc)?.get("attributes").cloned().unwrap_or(Value::Null);
    let name = key_name(&attrs)?;
    check_api_key_attrs(&attrs)?;
    let max = super::super::ingest_keys::MAX_KEYS;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM ingest_keys WHERE org_id = $1 AND revoked_at IS NULL").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= max {
        return Err(bad_request(format!("This organization already has {max} active keys. Revoke one first.")));
    }
    let key = crypto::random_token("ocik_");
    let info = super::super::ingest_keys::insert(&st, &ctx.org_id, &name, &key, Some(&ctx.user_id)).await?;
    ctx.audit(&st, "ingest_key.created", &info.prefix, &name).await;
    created(json!({ "data": api_key_doc(&find_api_key(&st, &ctx.org_id, &info.id).await?, Some(&key)) }))
}

/// Renames an API key (Datadog-compatible).
#[utoipa::path(patch, path = "/api/v2/api_keys/{api_key_id}", operation_id = "datadog_update_api_key", params(("api_key_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_api_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let data = data(&doc)?;
    mismatched_id(data, &id)?;
    let attrs = data.get("attributes").cloned().unwrap_or(Value::Null);
    let name = key_name(&attrs)?;
    check_api_key_attrs(&attrs)?;
    let key = find_api_key(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE ingest_keys SET name = $3 WHERE org_id = $1 AND id = $2::uuid AND revoked_at IS NULL").bind(&ctx.org_id).bind(&key.id).bind(&name).execute(&st.postgres).await?;
    if name != key.name {
        ctx.audit(&st, "ingest_key.renamed", &key.name, &name).await;
    }
    ok(json!({ "data": api_key_doc(&find_api_key(&st, &ctx.org_id, &id).await?, None) }))
}

/// Revokes an API key (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/api_keys/{api_key_id}", operation_id = "datadog_delete_api_key", params(("api_key_id" = String, Path)), responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_api_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let key = find_api_key(&st, &ctx.org_id, &id).await?;
    let row: Option<(String,)> = sqlx::query_as("UPDATE ingest_keys SET revoked_at = now() WHERE org_id = $1 AND id = $2::uuid AND revoked_at IS NULL RETURNING prefix")
        .bind(&ctx.org_id)
        .bind(&key.id)
        .fetch_optional(&st.postgres)
        .await?;
    let Some((prefix,)) = row else { return Err(not_found("API key not found")) };
    ctx.audit(&st, "ingest_key.revoked", &prefix, &key.name).await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ application keys

#[derive(sqlx::FromRow)]
struct AppKeyRow {
    id: String,
    user_id: String,
    name: String,
    prefix: String,
    created_at: i64,
    last_used_at: Option<i64>,
}

const APP_KEY_COLS: &str = "k.id::text AS id, k.user_id::text AS user_id, k.name, k.prefix, (extract(epoch FROM k.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM k.last_used_at) * 1000)::bigint AS last_used_at";

fn app_key_doc(k: &AppKeyRow, secret: Option<&str>) -> Value {
    let mut attrs = json!({
        "name": k.name, "created_at": iso(k.created_at), "last4": secret.map(last4).unwrap_or_default(), "scopes": null, "last_used_at": iso_opt(k.last_used_at),
    });
    if let Some(secret) = secret {
        attrs["key"] = json!(secret);
    }
    json!({
        "type": "application_keys",
        "id": k.id,
        "attributes": attrs,
        "relationships": { "owned_by": { "data": { "type": "users", "id": k.user_id } }, "leak_information": { "data": null } },
    })
}

/// Which application keys a request may see: the caller's own, or (admins) the organization's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Me,
    Org,
}

async fn find_app_key(st: &AppState, ctx: &AuthContext, owner: Owner, id: &str) -> Result<AppKeyRow, DdError> {
    let not_found = || not_found("Application key not found");
    let id = uuid(id).ok_or_else(not_found)?;
    let row: Option<AppKeyRow> = sqlx::query_as(&format!("SELECT {APP_KEY_COLS} FROM app_keys k WHERE k.org_id = $1 AND k.id = $2::uuid AND k.revoked_at IS NULL AND ($3 OR k.user_id = $4::uuid)"))
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(owner == Owner::Org)
        .bind(&ctx.user_id)
        .fetch_optional(&st.postgres)
        .await?;
    row.ok_or_else(not_found)
}

/// Our keys act with their user's role: scopes would promise a restriction we don't enforce.
fn reject_scopes(attrs: &Value) -> Result<(), DdError> {
    match attrs.get("scopes") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Array(s)) if s.is_empty() => Ok(()),
        Some(_) => Err(bad_request("Scoped application keys aren't supported by ObserveCraft: a key acts with its user's role. Leave `scopes` unset.")),
    }
}

/// One key with its owner in `included`.
async fn app_key_response(st: &AppState, ctx: &AuthContext, owner: Owner, id: &str) -> DocResult {
    let key = find_app_key(st, ctx, owner, id).await?;
    let included: Vec<Value> = super::users::find_user(st, &ctx.org_id, &key.user_id).await?.iter().map(|u| super::users::user_doc(u, &ctx.org_id)).collect();
    ok(json!({ "data": app_key_doc(&key, None), "included": included }))
}

async fn list_app_keys(st: &AppState, ctx: &AuthContext, owner: Owner, q: &HashMap<String, String>) -> DocResult {
    let page = DdPage::from(q)?;
    let filter = q.get("filter").map(|s| s.trim().to_string()).unwrap_or_default();
    let owned_by = q.get("filter[owned_by]").map(|s| s.trim().to_string()).unwrap_or_default();
    let order = sort_order(q, &[("name", "lower(k.name)"), ("created_at", "k.created_at"), ("last4", "k.created_at")])?;
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        key: AppKeyRow,
        filtered: i64,
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {APP_KEY_COLS}, count(*) OVER () AS filtered FROM app_keys k JOIN users u ON u.id = k.user_id \
         WHERE k.org_id = $1 AND k.revoked_at IS NULL AND ($2 OR k.user_id = $3::uuid) AND ($4 = '' OR k.name ILIKE $5) \
         AND ($6 = '' OR u.name ILIKE $7 OR u.email ILIKE $7) ORDER BY {order} LIMIT $8 OFFSET $9"
    ))
    .bind(&ctx.org_id)
    .bind(owner == Owner::Org)
    .bind(&ctx.user_id)
    .bind(&filter)
    .bind(ilike(&filter))
    .bind(&owned_by)
    .bind(ilike(&owned_by))
    .bind(page.size)
    .bind(page.offset())
    .fetch_all(&st.postgres)
    .await?;
    let filtered = rows.first().map_or(0, |r| r.filtered);
    ok(json!({
        "data": rows.iter().map(|r| app_key_doc(&r.key, None)).collect::<Vec<_>>(),
        "meta": { "page": { "total_filtered_count": filtered }, "max_allowed_per_user": MAX_APP_KEYS },
    }))
}

async fn rename_app_key(st: &AppState, ctx: &AuthContext, owner: Owner, id: &str, raw: &Bytes) -> DocResult {
    let doc = body(raw)?;
    let data = data(&doc)?;
    mismatched_id(data, id)?;
    let attrs = data.get("attributes").cloned().unwrap_or(Value::Null);
    reject_scopes(&attrs)?;
    let key = find_app_key(st, ctx, owner, id).await?;
    if let Some(name) = attrs.get("name").filter(|n| !n.is_null()) {
        let name = check_name(name.as_str().ok_or_else(|| bad_request("`name` must be a string"))?, "name", 100)?;
        sqlx::query("UPDATE app_keys SET name = $3 WHERE org_id = $1 AND id = $2::uuid AND revoked_at IS NULL").bind(&ctx.org_id).bind(&key.id).bind(&name).execute(&st.postgres).await?;
        if name != key.name {
            ctx.audit(st, "app_key.renamed", &key.prefix, &name).await;
        }
    }
    ok(json!({ "data": app_key_doc(&find_app_key(st, ctx, owner, id).await?, None) }))
}

async fn revoke_app_key(st: &AppState, ctx: &AuthContext, owner: Owner, id: &str) -> Result<StatusCode, DdError> {
    let key = find_app_key(st, ctx, owner, id).await?;
    let row: Option<(String,)> = sqlx::query_as("UPDATE app_keys SET revoked_at = now() WHERE org_id = $1 AND id = $2::uuid AND revoked_at IS NULL RETURNING prefix")
        .bind(&ctx.org_id)
        .bind(&key.id)
        .fetch_optional(&st.postgres)
        .await?;
    let Some((prefix,)) = row else { return Err(not_found("Application key not found")) };
    ctx.audit(st, "app_key.revoked", &prefix, if key.user_id == ctx.user_id { "" } else { "an organization admin revoked another user's key" }).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's application keys (Datadog-compatible), without their secrets.
#[utoipa::path(get, path = "/api/v2/current_user/application_keys", operation_id = "datadog_list_current_user_application_keys", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_my_app_keys(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    list_app_keys(&st, &ctx, Owner::Me, &q).await
}

/// Creates an application key (`ocak_…`) for the caller (Datadog-compatible); the secret is in this answer only.
#[utoipa::path(post, path = "/api/v2/current_user/application_keys", operation_id = "datadog_create_current_user_application_key", request_body = super::users::DatadogIamDocument, responses((status = 201, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_my_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    if !st.config.datadog_api_allow_app_key_creation {
        return Err(DdError::new(
            StatusCode::FORBIDDEN,
            "Creating application keys with an application key is turned off on this server. Create the key in ObserveCraft (Organization Settings → Application Keys), or set OC_DATADOG_API_ALLOW_APP_KEY_CREATION=true to let the Datadog-compatible API (e.g. Terraform's datadog_application_key) create them.",
        ));
    }
    let doc = body(&raw)?;
    let attrs = data(&doc)?.get("attributes").cloned().unwrap_or(Value::Null);
    let name = key_name(&attrs)?;
    reject_scopes(&attrs)?;
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM app_keys WHERE org_id = $1 AND user_id = $2::uuid AND revoked_at IS NULL").bind(&ctx.org_id).bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    if count >= MAX_APP_KEYS {
        return Err(bad_request(format!("You already have {MAX_APP_KEYS} active keys. Revoke one first.")));
    }
    let key = crypto::random_token("ocak_");
    let prefix: String = key.chars().take(10).collect();
    let (id,): (String,) = sqlx::query_as("INSERT INTO app_keys (org_id, user_id, name, key_hash, prefix) VALUES ($1, $2::uuid, $3, $4, $5) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .bind(&name)
        .bind(crypto::sha256(&key))
        .bind(&prefix)
        .fetch_one(&st.postgres)
        .await?;
    ctx.audit(&st, "app_key.created", &prefix, &format!("{name} (Datadog-compatible API)")).await;
    created(json!({ "data": app_key_doc(&find_app_key(&st, &ctx, Owner::Me, &id).await?, Some(&key)) }))
}

/// One of the caller's application keys (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/current_user/application_keys/{app_key_id}", operation_id = "datadog_get_current_user_application_key", params(("app_key_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_my_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    app_key_response(&st, &ctx, Owner::Me, &id).await
}

/// Renames one of the caller's application keys (Datadog-compatible).
#[utoipa::path(patch, path = "/api/v2/current_user/application_keys/{app_key_id}", operation_id = "datadog_update_current_user_application_key", params(("app_key_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_my_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    rename_app_key(&st, &ctx, Owner::Me, &id, &raw).await
}

/// Revokes one of the caller's application keys (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/current_user/application_keys/{app_key_id}", operation_id = "datadog_delete_current_user_application_key", params(("app_key_id" = String, Path)), responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_my_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    revoke_app_key(&st, &ctx, Owner::Me, &id).await
}

/// Every application key of the organization (Datadog-compatible, admins): `filter`, `filter[owned_by]`, paging and sort.
#[utoipa::path(get, path = "/api/v2/application_keys", operation_id = "datadog_list_application_keys", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list_org_app_keys(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<HashMap<String, String>>) -> DocResult {
    require(&ctx, Access::Admin)?;
    list_app_keys(&st, &ctx, Owner::Org, &q).await
}

/// Any application key of the organization (Datadog-compatible, admins).
#[utoipa::path(get, path = "/api/v2/application_keys/{app_key_id}", operation_id = "datadog_get_application_key", params(("app_key_id" = String, Path)), responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_org_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    app_key_response(&st, &ctx, Owner::Org, &id).await
}

/// Renames any application key of the organization (Datadog-compatible, admins).
#[utoipa::path(patch, path = "/api/v2/application_keys/{app_key_id}", operation_id = "datadog_update_application_key", params(("app_key_id" = String, Path)), request_body = super::users::DatadogIamDocument, responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_org_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    rename_app_key(&st, &ctx, Owner::Org, &id, &raw).await
}

/// Revokes any application key of the organization (Datadog-compatible, admins).
#[utoipa::path(delete, path = "/api/v2/application_keys/{app_key_id}", operation_id = "datadog_delete_application_key", params(("app_key_id" = String, Path)), responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_org_app_key(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    revoke_app_key(&st, &ctx, Owner::Org, &id).await
}

#[cfg(test)]
mod tests {
    use super::super::replay::{user_with_app_key, Feature, TestServer};
    use super::super::users::tests::send;
    use super::*;
    use reqwest::Method;

    #[tokio::test]
    async fn api_keys_are_ingest_keys_shown_once() {
        let Some(s) = TestServer::start("dd-api-keys").await else { return };
        let (code, created) = send(&s, &s.app_key, Method::POST, "/api/v2/api_keys", Some(json!({ "data": { "type": "api_keys", "attributes": { "name": "storefront agents" } } }))).await;
        assert_eq!(code, 201, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_string();
        let secret = created["data"]["attributes"]["key"].as_str().unwrap().to_string();
        assert!(secret.starts_with("ocik_"));
        assert_eq!(created["data"]["attributes"]["last4"], json!(last4(&secret)));
        // A working ingest key of the org, stored hashed.
        assert_eq!(super::super::ingest_key_org(&s.st, &secret).await.unwrap().as_deref(), Some(s.org.as_str()));
        let (stored,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM ingest_keys WHERE org_id = $1 AND key_hash = $2").bind(&s.org).bind(crypto::sha256(&secret)).fetch_one(&s.st.postgres).await.unwrap();
        assert_eq!(stored, 1);

        let (code, got) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/api_keys/{id}"), None).await;
        assert_eq!(code, 200);
        assert!(got["data"]["attributes"].get("key").is_none(), "never shown again: {got}");
        let (code, renamed) =
            send(&s, &s.app_key, Method::PATCH, &format!("/api/v2/api_keys/{id}"), Some(json!({ "data": { "type": "api_keys", "id": id, "attributes": { "name": "storefront agents (eu)" } } }))).await;
        assert_eq!((code, renamed["data"]["attributes"]["name"].as_str()), (200, Some("storefront agents (eu)")));
        let (_, list) = send(&s, &s.app_key, Method::GET, "/api/v2/api_keys?filter=storefront", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");

        let (code, err) =
            send(&s, &s.app_key, Method::POST, "/api/v2/api_keys", Some(json!({ "data": { "type": "api_keys", "attributes": { "name": "rc", "remote_config_read_enabled": true } } }))).await;
        assert_eq!(code, 400, "{err}");
        let (_, standard_key) = user_with_app_key(&s.st, &s.org, "standard").await;
        let (code, _) = send(&s, &standard_key, Method::POST, "/api/v2/api_keys", Some(json!({ "data": { "type": "api_keys", "attributes": { "name": "mine" } } }))).await;
        assert_eq!(code, 403, "ingest keys are for admins");
        let (code, _) = send(&s, &standard_key, Method::DELETE, &format!("/api/v2/api_keys/{id}"), None).await;
        assert_eq!(code, 403);
        let other = TestServer::start("dd-api-keys-other").await.unwrap();
        let (code, _) = send(&other, &other.app_key, Method::DELETE, &format!("/api/v2/api_keys/{id}"), None).await;
        assert_eq!(code, 404, "another org's key");

        let (code, _) = send(&s, &s.app_key, Method::DELETE, &format!("/api/v2/api_keys/{id}"), None).await;
        assert_eq!(code, 204);
        assert_eq!(super::super::ingest_key_org(&s.st, &secret).await.unwrap(), None, "revoked");
        let (code, _) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/api_keys/{id}"), None).await;
        assert_eq!(code, 404);
    }

    #[tokio::test]
    async fn application_keys_through_the_api_need_the_setting() {
        // Off by default: a leaked key must not be able to mint keys that outlive its revocation.
        let Some(s) = TestServer::start("dd-app-keys-off").await else { return };
        let (code, err) =
            send(&s, &s.app_key, Method::POST, "/api/v2/current_user/application_keys", Some(json!({ "data": { "type": "application_keys", "attributes": { "name": "ci" } } }))).await;
        assert_eq!(code, 403, "{err}");
        assert!(err["errors"][0].as_str().unwrap_or_default().contains("OC_DATADOG_API_ALLOW_APP_KEY_CREATION"), "{err}");
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM app_keys WHERE org_id = $1").bind(&s.org).fetch_one(&s.st.postgres).await.unwrap();
        assert_eq!(count, 1, "only the test's own key exists");
    }

    #[tokio::test]
    async fn application_keys_belong_to_their_user() {
        let Some(s) = TestServer::start_with("dd-app-keys", |c| c.datadog_api_allow_app_key_creation = true).await else { return };
        let (member, member_key) = user_with_app_key(&s.st, &s.org, "readonly").await;
        let (code, created) =
            send(&s, &member_key, Method::POST, "/api/v2/current_user/application_keys", Some(json!({ "data": { "type": "application_keys", "attributes": { "name": "ci" } } }))).await;
        assert_eq!(code, 201, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_string();
        let secret = created["data"]["attributes"]["key"].as_str().unwrap().to_string();
        assert!(secret.starts_with("ocak_"));
        assert_eq!(created["data"]["attributes"]["scopes"], Value::Null);
        assert_eq!(created["data"]["relationships"]["owned_by"]["data"]["id"], json!(member));
        // The new key acts as the same (read-only) user: no privilege gained.
        let (code, _) = send(&s, &secret, Method::POST, "/api/v2/api_keys", Some(json!({ "data": { "type": "api_keys", "attributes": { "name": "x" } } }))).await;
        assert_eq!(code, 403);
        let (code, me) = send(&s, &secret, Method::GET, "/api/v2/current_user", None).await;
        assert_eq!((code, me["data"]["id"].as_str()), (200, Some(member.as_str())));

        let (code, err) = send(
            &s,
            &member_key,
            Method::POST,
            "/api/v2/current_user/application_keys",
            Some(json!({ "data": { "type": "application_keys", "attributes": { "name": "scoped", "scopes": ["dashboards_read"] } } })),
        )
        .await;
        assert_eq!(code, 400, "{err}");
        let (code, _) = send(&s, &member_key, Method::GET, &format!("/api/v2/application_keys/{id}"), None).await;
        assert_eq!(code, 403, "the organization's keys are for admins");
        let (code, got) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/application_keys/{id}"), None).await;
        assert_eq!(code, 200, "{got}");
        let (code, _) = send(&s, &s.app_key, Method::GET, &format!("/api/v2/current_user/application_keys/{id}"), None).await;
        assert_eq!(code, 404, "not the admin's own key");
        let (_, mine) = send(&s, &member_key, Method::GET, "/api/v2/current_user/application_keys", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 2, "{mine}");

        let (code, renamed) = send(
            &s,
            &member_key,
            Method::PATCH,
            &format!("/api/v2/current_user/application_keys/{id}"),
            Some(json!({ "data": { "type": "application_keys", "id": id, "attributes": { "name": "ci (nightly)" } } })),
        )
        .await;
        assert_eq!((code, renamed["data"]["attributes"]["name"].as_str()), (200, Some("ci (nightly)")));
        let (code, _) = send(&s, &member_key, Method::DELETE, &format!("/api/v2/current_user/application_keys/{id}"), None).await;
        assert_eq!(code, 204);
        let (code, _) = send(&s, &secret, Method::GET, "/api/v2/current_user", None).await;
        assert_eq!(code, 403, "revoked");
        let (code, _) = send(&s, &member_key, Method::DELETE, &format!("/api/v2/current_user/application_keys/{id}"), None).await;
        assert_eq!(code, 404);
    }

    #[tokio::test]
    async fn replays_datadogs_key_management_scenarios() {
        let Some(s) = TestServer::start_with("dd-keys-replay", |c| c.datadog_api_allow_app_key_creation = true).await else { return };
        let f = Feature::load("v2/key_management");
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            // Datadog shows API keys again on reads; we keep only their hash, so `key` is in the create answer only.
            problems.extend(p.into_iter().filter(|m| m.contains(" POST ") || !m.ends_with("missing `data.attributes.key`")));
            replayed += 1;
        }
        assert!(replayed >= 17, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
