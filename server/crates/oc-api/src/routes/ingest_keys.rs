//! Ingest API keys: what applications, collectors and agents send telemetry with. The intake
//! gateway (`oc-intake`) resolves a key to its organization; revoking a key stops ingestion within
//! the gateway's cache lifetime (60 s). Org admins manage them; every change is audited.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{crypto, Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Active keys an org may hold at once.
pub(crate) const MAX_KEYS: i64 = 50;

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct IngestKey {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_by: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    /// Set once revoked: the key no longer authenticates.
    pub revoked_at: Option<i64>,
}

const KEY_SELECT: &str = "SELECT k.id::text, k.name, k.prefix, u.name AS created_by, (extract(epoch FROM k.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM k.last_used_at) * 1000)::bigint AS last_used_at, (extract(epoch FROM k.revoked_at) * 1000)::bigint AS revoked_at FROM ingest_keys k LEFT JOIN users u ON u.id = k.created_by";

/// Every key of the organization, revoked ones included (with `revokedAt`), newest first.
#[utoipa::path(get, path = "/api/v1/ingest-keys", operation_id = "list_ingest_keys", responses((status = 200, body = Vec<IngestKey>)), tag = "ingest")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<IngestKey>> {
    let rows = sqlx::query_as(&format!("{KEY_SELECT} WHERE k.org_id = $1 ORDER BY k.revoked_at IS NOT NULL, k.created_at DESC"))
        .bind(&ctx.org_id)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateIngestKey {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedIngestKey {
    #[serde(flatten)]
    pub info: IngestKey,
    /// The secret. Shown only once.
    pub key: String,
}

/// Creates a key in `org`; returns the secret. Also used to seed the local demo key.
pub(crate) async fn insert(st: &AppState, org: &str, name: &str, key: &str, created_by: Option<&str>) -> Result<IngestKey, ApiError> {
    let prefix: String = key.chars().take(10).collect();
    let (id,): (String,) = sqlx::query_as("INSERT INTO ingest_keys (org_id, name, key_hash, prefix, created_by) VALUES ($1, $2, $3, $4, $5::uuid) RETURNING id::text")
        .bind(org)
        .bind(name)
        .bind(crypto::sha256(key))
        .bind(&prefix)
        .bind(created_by)
        .fetch_one(&st.postgres)
        .await?;
    Ok(sqlx::query_as(&format!("{KEY_SELECT} WHERE k.id = $1::uuid")).bind(&id).fetch_one(&st.postgres).await?)
}

#[utoipa::path(post, path = "/api/v1/ingest-keys", operation_id = "create_ingest_key", request_body = CreateIngestKey, responses((status = 200, body = CreatedIngestKey)), tag = "ingest")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<CreateIngestKey>) -> ApiResult<CreatedIngestKey> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest("name must be 1–100 characters".into()));
    }
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM ingest_keys WHERE org_id = $1 AND revoked_at IS NULL").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_KEYS {
        return Err(ApiError::BadRequest(format!("This organization already has {MAX_KEYS} active keys. Revoke one first.")));
    }
    let key = crypto::random_token("ocik_");
    let info = insert(&st, &ctx.org_id, name, &key, Some(&ctx.user_id)).await?;
    ctx.audit(&st, "ingest_key.created", &info.prefix, name).await;
    Ok(Json(CreatedIngestKey { info, key }))
}

#[utoipa::path(delete, path = "/api/v1/ingest-keys/{id}", operation_id = "revoke_ingest_key", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "ingest")]
pub async fn revoke(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String, String)> = sqlx::query_as("UPDATE ingest_keys SET revoked_at = now() WHERE org_id = $1 AND id::text = $2 AND revoked_at IS NULL RETURNING prefix, name")
        .bind(&ctx.org_id)
        .bind(&id)
        .fetch_optional(&st.postgres)
        .await?;
    let Some((prefix, name)) = row else { return Err(ApiError::NotFound("Key not found".into())) };
    ctx.audit(&st, "ingest_key.revoked", &prefix, &name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Development only (`OC_SEED_DEMO`): makes `OC_DEMO_APP_KEY` an application key of the dev user, so
/// oc-tracegen can call AI Guard's evaluate endpoint as the tracers' SDKs do.
pub async fn ensure_demo_app_key(st: &AppState) -> anyhow::Result<()> {
    let Some(key) = st.config.demo_app_key.as_deref() else { return Ok(()) };
    let exists: Option<(String,)> = sqlx::query_as("SELECT org_id FROM app_keys WHERE key_hash = $1").bind(crypto::sha256(key)).fetch_optional(&st.postgres).await?;
    if exists.is_none() {
        let prefix: String = key.chars().take(10).collect();
        sqlx::query("INSERT INTO app_keys (org_id, user_id, name, key_hash, prefix) SELECT $1, id, 'Local demo generators', $3, $4 FROM users WHERE org_id = $1 AND id = $2::uuid")
            .bind(&st.config.dev_org)
            .bind(&st.config.dev_user)
            .bind(crypto::sha256(key))
            .bind(&prefix)
            .execute(&st.postgres)
            .await?;
        tracing::info!("created the demo application key (OC_DEMO_APP_KEY) for the dev user");
    }
    Ok(())
}

/// Development only (`OC_SEED_DEMO`): makes `OC_DEMO_INGEST_KEY` a key of the install's org, so the
/// synthetic data generators can send through the intake without a manual step.
pub async fn ensure_demo_key(st: &AppState) -> anyhow::Result<()> {
    let Some(key) = st.config.demo_ingest_key.as_deref() else { return Ok(()) };
    let exists: Option<(String,)> = sqlx::query_as("SELECT org_id FROM ingest_keys WHERE key_hash = $1").bind(crypto::sha256(key)).fetch_optional(&st.postgres).await?;
    if exists.is_none() {
        insert(st, &st.config.dev_org, "Local demo generators", key, None).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        tracing::info!("created the demo ingest key (OC_DEMO_INGEST_KEY) for the install's org");
    }
    Ok(())
}
