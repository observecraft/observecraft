//! Standard attributes (Log Configuration → Standard Attributes): the org's list (the defaults with
//! its changes) and its edits. The indexer (`logproc`) reloads them every 10 s and remaps aliases on
//! every ingested log; the list, defaults and remapping are in `crate::standard_attributes`.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::standard_attributes::{self as sa, Override, StandardAttribute, StandardAttributeInput, StoredRow};
use crate::state::AppState;

/// The org's changes to the defaults.
async fn overrides(st: &AppState, org: &str) -> Result<Vec<Override>, sqlx::Error> {
    let rows: Vec<StoredRow> =
        sqlx::query_as("SELECT org_id, path, hidden, type, description, aliases, preserve_source FROM log_standard_attributes WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(Override::from).collect())
}

/// The org's standard attributes: the defaults it kept (with its edits) and the ones it added.
pub async fn org_list(st: &AppState, org: &str) -> Result<Vec<StandardAttribute>, sqlx::Error> {
    Ok(sa::merge(&overrides(st, org).await?))
}

/// The paths of the org's standard attributes (facets mark them).
pub async fn org_paths(st: &AppState, org: &str) -> Result<HashSet<String>, sqlx::Error> {
    Ok(org_list(st, org).await?.into_iter().map(|a| a.path).collect())
}

/// Stores the org's definition of an attribute (shown instead of a default with the same path).
async fn save(st: &AppState, org: &str, a: &StandardAttributeInput) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO log_standard_attributes (org_id, path, hidden, type, description, aliases, preserve_source) VALUES ($1, $2, false, $3, $4, $5, $6) \
         ON CONFLICT (org_id, path) DO UPDATE SET hidden = false, type = $3, description = $4, aliases = $5, preserve_source = $6, updated_at = now()",
    )
    .bind(org)
    .bind(&a.path)
    .bind(a.kind.as_str())
    .bind(&a.description)
    .bind(serde_json::to_value(&a.aliases).unwrap_or_default())
    .bind(a.preserve_source)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

/// Validates `input` against the org's other attributes.
fn checked(input: &mut StandardAttributeInput, list: &[StandardAttribute], path: &str) -> Result<(), ApiError> {
    let others: Vec<StandardAttribute> = list.iter().filter(|a| a.path != path).cloned().collect();
    sa::validate(input, &others).map_err(ApiError::BadRequest)
}

fn find(list: Vec<StandardAttribute>, path: &str) -> Result<StandardAttribute, ApiError> {
    list.into_iter().find(|a| a.path == path).ok_or_else(|| ApiError::NotFound(format!("No standard attribute '{path}'")))
}

#[utoipa::path(get, path = "/api/v1/logs/standard-attributes", operation_id = "list_standard_attributes", responses((status = 200, body = Vec<StandardAttribute>)), tag = "logs")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<StandardAttribute>> {
    Ok(Json(org_list(&st, &ctx.org_id).await?))
}

/// Adds an attribute (or brings back a default the org removed, with this definition).
#[utoipa::path(post, path = "/api/v1/logs/standard-attributes", operation_id = "create_standard_attribute", request_body = StandardAttributeInput, responses((status = 200, body = StandardAttribute)), tag = "logs")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<StandardAttributeInput>) -> ApiResult<StandardAttribute> {
    let list = org_list(&st, &ctx.org_id).await?;
    let path = input.path.trim().trim_start_matches('@').to_string();
    if list.iter().any(|a| a.path == path) {
        return Err(ApiError::BadRequest(format!("'{path}' is already a standard attribute")));
    }
    if list.len() >= sa::MAX_ATTRIBUTES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {} standard attributes", sa::MAX_ATTRIBUTES)));
    }
    checked(&mut input, &list, &path)?;
    save(&st, &ctx.org_id, &input).await?;
    ctx.audit(&st, "standard_attribute.created", &input.path, &input.aliases.join(", ")).await;
    Ok(Json(find(org_list(&st, &ctx.org_id).await?, &input.path)?))
}

/// Changes the type, description, aliases or source handling; the path stays.
#[utoipa::path(put, path = "/api/v1/logs/standard-attributes/{path}", operation_id = "update_standard_attribute", params(("path" = String, Path)), request_body = StandardAttributeInput, responses((status = 200, body = StandardAttribute)), tag = "logs")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(path): Path<String>, Json(mut input): Json<StandardAttributeInput>) -> ApiResult<StandardAttribute> {
    let list = org_list(&st, &ctx.org_id).await?;
    find(list.clone(), &path)?;
    if input.path.trim().trim_start_matches('@') != path {
        return Err(ApiError::BadRequest("A standard attribute can't be renamed: add a new one".into()));
    }
    checked(&mut input, &list, &path)?;
    save(&st, &ctx.org_id, &input).await?;
    ctx.audit(&st, "standard_attribute.updated", &path, &input.aliases.join(", ")).await;
    Ok(Json(find(org_list(&st, &ctx.org_id).await?, &path)?))
}

/// Removes an attribute; a default is hidden until the list is reset. Logs already stored keep their attributes.
#[utoipa::path(delete, path = "/api/v1/logs/standard-attributes/{path}", operation_id = "delete_standard_attribute", params(("path" = String, Path)), responses((status = 204)), tag = "logs")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(path): Path<String>) -> Result<StatusCode, ApiError> {
    let attribute = find(org_list(&st, &ctx.org_id).await?, &path)?;
    if attribute.is_default {
        sqlx::query(
            "INSERT INTO log_standard_attributes (org_id, path, hidden) VALUES ($1, $2, true) \
             ON CONFLICT (org_id, path) DO UPDATE SET hidden = true, updated_at = now()",
        )
        .bind(&ctx.org_id)
        .bind(&path)
        .execute(&st.postgres)
        .await?;
    } else {
        sqlx::query("DELETE FROM log_standard_attributes WHERE org_id = $1 AND path = $2").bind(&ctx.org_id).bind(&path).execute(&st.postgres).await?;
    }
    ctx.audit(&st, "standard_attribute.deleted", &path, "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Drops the org's changes: the list is the defaults again.
#[utoipa::path(delete, path = "/api/v1/logs/standard-attributes", operation_id = "reset_standard_attributes", responses((status = 200, body = Vec<StandardAttribute>)), tag = "logs")]
pub async fn reset(State(st): State<AppState>, Writer(ctx): Writer) -> ApiResult<Vec<StandardAttribute>> {
    sqlx::query("DELETE FROM log_standard_attributes WHERE org_id = $1").bind(&ctx.org_id).execute(&st.postgres).await?;
    ctx.audit(&st, "standard_attributes.reset", "", "").await;
    Ok(Json(org_list(&st, &ctx.org_id).await?))
}
