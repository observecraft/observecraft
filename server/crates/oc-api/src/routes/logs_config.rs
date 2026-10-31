//! Logs → Configuration, the parts beyond the per-feature modules: custom destinations (the UI's
//! side of [`crate::routes::datadog_api::logs::destinations`], same records), Flex Logs controls,
//! and the pipelines' usage (share of the last hour's logs each one's filter matches).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::datadog_api::logs::destinations::{find, input, insert, save, secrets_of};
use super::datadog_api::DdError;
use super::logs::{bind, scope, TimeRange};
use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::log_forwarding::{self, Destination, SENTINEL_UNSUPPORTED};
use crate::state::AppState;

impl From<DdError> for ApiError {
    fn from(e: DdError) -> Self {
        let m = e.errors.join(": ");
        match e.status {
            StatusCode::NOT_FOUND => ApiError::NotFound(m),
            StatusCode::CONFLICT => ApiError::Conflict(m),
            StatusCode::FORBIDDEN => ApiError::Forbidden(m),
            _ => ApiError::BadRequest(m),
        }
    }
}

// ================================================================ custom destinations

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomDestination {
    pub id: String,
    pub name: String,
    /// Logs matching this search are forwarded; '' = every log.
    pub query: String,
    pub enabled: bool,
    pub forward_tags: bool,
    /// Tag keys kept (`ALLOW_LIST`) or dropped (`BLOCK_LIST`); empty = every tag.
    pub tags_list: Vec<String>,
    pub tags_list_type: String,
    /// Datadog's `forwarder_destination` without its secrets (`type`: http, splunk_hec, elasticsearch, microsoft_sentinel).
    #[schema(value_type = Object)]
    pub destination: Value,
    /// Logs forwarded since it was created.
    pub forwarded: i64,
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
    pub created_by: Option<String>,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomDestinationInput {
    pub name: String,
    #[serde(default)]
    pub query: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub forward_tags: bool,
    #[serde(default)]
    pub tags_list: Vec<String>,
    #[serde(default = "allow")]
    pub tags_list_type: String,
    /// Datadog's `forwarder_destination`, secrets included (left out on a change: kept).
    #[schema(value_type = Object)]
    pub destination: Value,
}

fn yes() -> bool {
    true
}
fn allow() -> String {
    "ALLOW_LIST".into()
}

#[derive(sqlx::FromRow)]
struct Extra {
    forwarded: i64,
    last_error: Option<String>,
    last_error_at: Option<i64>,
    created_by: Option<String>,
    updated_at: i64,
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<CustomDestination, ApiError> {
    let r = find(st, org, id).await?;
    let x: Extra = sqlx::query_as(
        "SELECT d.forwarded, d.last_error, (extract(epoch FROM d.last_error_at) * 1000)::bigint AS last_error_at, u.name AS created_by, \
         (extract(epoch FROM d.updated_at) * 1000)::bigint AS updated_at FROM log_custom_destinations d LEFT JOIN users u ON u.id = d.created_by \
         WHERE d.org_id = $1 AND d.id::text = $2",
    )
    .bind(org)
    .bind(&r.id)
    .fetch_one(&st.postgres)
    .await?;
    let sentinel = r.destination.get("type").and_then(Value::as_str) == Some("microsoft_sentinel");
    Ok(CustomDestination {
        id: r.id,
        name: r.name,
        query: r.query,
        enabled: r.enabled,
        forward_tags: r.forward_tags,
        tags_list: serde_json::from_value(r.tags_list).unwrap_or_default(),
        tags_list_type: r.tags_list_type,
        destination: r.destination,
        forwarded: x.forwarded,
        last_error: if sentinel { Some(SENTINEL_UNSUPPORTED.into()) } else { x.last_error },
        last_error_at: x.last_error_at,
        created_by: x.created_by,
        updated_at: x.updated_at,
    })
}

fn attributes(i: &CustomDestinationInput) -> Value {
    json!({
        "name": i.name, "query": i.query, "enabled": i.enabled, "forward_tags": i.forward_tags,
        "forward_tags_restriction_list": i.tags_list, "forward_tags_restriction_list_type": i.tags_list_type,
        "forwarder_destination": i.destination,
    })
}

#[utoipa::path(get, path = "/api/v1/logs/custom-destinations", operation_id = "list_log_custom_destinations", responses((status = 200, body = Vec<CustomDestination>)), tag = "logs")]
pub async fn list_destinations(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<CustomDestination>> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id::text FROM log_custom_destinations WHERE org_id = $1 ORDER BY position, created_at").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(load(&st, &ctx.org_id, &id).await?);
    }
    Ok(Json(out))
}

#[utoipa::path(post, path = "/api/v1/logs/custom-destinations", operation_id = "create_log_custom_destination", request_body = CustomDestinationInput,
    responses((status = 200, body = CustomDestination)), tag = "logs")]
pub async fn create_destination(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<CustomDestinationInput>) -> ApiResult<CustomDestination> {
    let i = input(&attributes(&req), None)?;
    let id = insert(&st, &ctx.org_id, &ctx.user_id, &i).await?;
    ctx.audit(&st, "log_destination.created", &i.name, &i.query).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/logs/custom-destinations/{id}", operation_id = "update_log_custom_destination", params(("id" = String, Path)),
    request_body = CustomDestinationInput, responses((status = 200, body = CustomDestination)), tag = "logs")]
pub async fn update_destination(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(req): Json<CustomDestinationInput>) -> ApiResult<CustomDestination> {
    let current = find(&st, &ctx.org_id, &id).await?;
    let old = secrets_of(&st, &current);
    let i = input(&attributes(&req), Some((&current, old)))?;
    save(&st, &ctx.org_id, &current.id, &i).await?;
    ctx.audit(&st, "log_destination.updated", &i.name, &i.query).await;
    Ok(Json(load(&st, &ctx.org_id, &current.id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct DestinationEnabled {
    pub enabled: bool,
}

#[utoipa::path(patch, path = "/api/v1/logs/custom-destinations/{id}", operation_id = "set_log_custom_destination_enabled", params(("id" = String, Path)),
    request_body = DestinationEnabled, responses((status = 200, body = CustomDestination)), tag = "logs")]
pub async fn set_destination_enabled(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(p): Json<DestinationEnabled>) -> ApiResult<CustomDestination> {
    let r = find(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE log_custom_destinations SET enabled = $3, updated_at = now() WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&r.id).bind(p.enabled).execute(&st.postgres).await?;
    ctx.audit(&st, "log_destination.updated", &r.name, &format!("enabled={}", p.enabled)).await;
    Ok(Json(load(&st, &ctx.org_id, &r.id).await?))
}

#[utoipa::path(delete, path = "/api/v1/logs/custom-destinations/{id}", operation_id = "delete_log_custom_destination", params(("id" = String, Path)), responses((status = 204)), tag = "logs")]
pub async fn delete_destination(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let r = find(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_custom_destinations WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&r.id).execute(&st.postgres).await?;
    ctx.audit(&st, "log_destination.deleted", &r.name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct DestinationTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

/// Sends one test log to the destination.
#[utoipa::path(post, path = "/api/v1/logs/custom-destinations/{id}/test", operation_id = "test_log_custom_destination", params(("id" = String, Path)),
    responses((status = 200, body = DestinationTestResult)), tag = "logs")]
pub async fn test_destination(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> ApiResult<DestinationTestResult> {
    let r = find(&st, &ctx.org_id, &id).await?;
    let secrets = Value::Object(secrets_of(&st, &r));
    let target = Destination::target(&r.destination, &secrets);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let log = json!({ "message": "Test log from ObserveCraft custom destination", "status": "info", "service": "observecraft", "ddsource": "observecraft", "timestamp": now });
    Ok(Json(match log_forwarding::send(&st, &target, &[log]).await {
        Ok(()) => DestinationTestResult { ok: true, error: None },
        Err(e) => DestinationTestResult { ok: false, error: Some(e) },
    }))
}

// ================================================================ Flex Logs

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlexSettings {
    /// Indexes keep logs for their flex retention (standard + flex tier).
    pub enabled: bool,
    /// `starter` or `scalable`.
    pub compute: String,
    /// With scalable compute: xs, xs+, s, m or l.
    pub size: Option<String>,
    /// Indexes with a flex retention.
    pub flex_indexes: i64,
    /// Longest flex retention allowed, in days.
    pub max_flex_days: i32,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlexSettingsInput {
    pub enabled: bool,
    pub compute: String,
    #[serde(default)]
    pub size: Option<String>,
}

async fn flex(st: &AppState, org: &str) -> Result<FlexSettings, ApiError> {
    let row: Option<(bool, String, Option<String>)> = sqlx::query_as("SELECT flex_enabled, flex_compute, flex_size FROM org_logs_settings WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    let (enabled, compute, size) = row.unwrap_or((false, "starter".into(), None));
    let (flex_indexes,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_indexes WHERE org_id = $1 AND flex_retention_days IS NOT NULL").bind(org).fetch_one(&st.postgres).await?;
    Ok(FlexSettings { enabled, compute, size, flex_indexes, max_flex_days: super::log_indexes::MAX_FLEX_DAYS })
}

#[utoipa::path(get, path = "/api/v1/logs/flex-settings", operation_id = "get_log_flex_settings", responses((status = 200, body = FlexSettings)), tag = "logs")]
pub async fn get_flex(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<FlexSettings> {
    Ok(Json(flex(&st, &ctx.org_id).await?))
}

/// Turns the flex tier on or off (applies to logs indexed from then on) and picks the compute.
#[utoipa::path(put, path = "/api/v1/logs/flex-settings", operation_id = "update_log_flex_settings", request_body = FlexSettingsInput, responses((status = 200, body = FlexSettings)), tag = "logs")]
pub async fn update_flex(State(st): State<AppState>, Admin(ctx): Admin, Json(i): Json<FlexSettingsInput>) -> ApiResult<FlexSettings> {
    let size = match (i.compute.as_str(), i.size.as_deref()) {
        ("starter", _) => None,
        ("scalable", Some(s @ ("xs" | "xs+" | "s" | "m" | "l"))) => Some(s.to_string()),
        ("scalable", _) => return Err(ApiError::BadRequest("Scalable compute needs a size: xs, xs+, s, m or l".into())),
        _ => return Err(ApiError::BadRequest("Compute is starter or scalable".into())),
    };
    sqlx::query(
        "INSERT INTO org_logs_settings (org_id, flex_enabled, flex_compute, flex_size, updated_by) VALUES ($1, $2, $3, $4, $5::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET flex_enabled = $2, flex_compute = $3, flex_size = $4, updated_by = $5::uuid, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(i.enabled)
    .bind(&i.compute)
    .bind(&size)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "logs.flex_changed", "", &format!("enabled={} compute={} size={size:?}", i.enabled, i.compute)).await;
    Ok(Json(flex(&st, &ctx.org_id).await?))
}

// ================================================================ pipeline usage

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineUsage {
    pub id: String,
    /// Indexed logs of the last hour matching the pipeline's filter.
    pub matched: u64,
    /// Share of the last hour's indexed logs (0–1).
    pub share: f64,
}

#[utoipa::path(get, path = "/api/v1/logs/pipelines/usage", operation_id = "log_pipelines_usage", responses((status = 200, body = Vec<PipelineUsage>)), tag = "logs")]
pub async fn pipelines_usage(State(st): State<AppState>, ctx: AuthContext, access: LogAccess) -> ApiResult<Vec<PipelineUsage>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id::text, filter FROM log_pipelines WHERE org_id = $1 AND enabled ORDER BY position").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let range = TimeRange { from: now - 3_600_000, to: now };
    let count = |filter: String| {
        let (st, org, access) = (st.clone(), ctx.org_id.clone(), access.clone());
        async move {
            let s = scope(&org, &access, range, &oc_query::parse_query(&filter), None);
            bind(st.ch(&org).query(&format!("SELECT count() FROM {} WHERE {}", s.table, s.clause)), &s.params).fetch_one::<u64>().await.unwrap_or(0)
        }
    };
    let total = count(String::new()).await.max(1);
    let counts = join_all(rows.iter().map(|(_, f)| count(f.clone()))).await;
    Ok(Json(rows.into_iter().zip(counts).map(|((id, _), matched)| PipelineUsage { id, matched, share: matched as f64 / total as f64 }).collect()))
}
