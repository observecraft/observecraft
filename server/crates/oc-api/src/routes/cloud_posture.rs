//! Cloud Security → account scans (docs/adr/0006): each cloud account's latest posture scan, and a scan on demand.
//! The scans themselves are `crate::posture`; their findings appear in Cloud Security → Findings
//! (`cloud_security::misconfigurations`) and Datadog's `/api/v2/security/findings`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::posture::store;
use crate::state::AppState;

/// A cloud account and its latest posture scan.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountScan {
    pub provider: String,
    /// The integration's account id (`aws_accounts.id`).
    pub account_ref: String,
    pub account_id: String,
    /// Posture collection (AWS `cloud_security_posture_management_collection`) is on.
    pub enabled: bool,
    /// queued | running | done | failed; None before the first scan.
    pub status: Option<String>,
    pub queued_at: Option<i64>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
    /// Prowler's version that ran.
    pub tool_version: Option<String>,
    pub passed: Option<i32>,
    pub failed: Option<i32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PostureScans {
    /// A scanner can take scans (`OC_SCANNER_TOKEN` is set).
    pub scanner_configured: bool,
    pub interval_hours: i64,
    pub accounts: Vec<AccountScan>,
}

#[utoipa::path(get, path = "/api/v1/cloud-security/scans", operation_id = "cloud_security_scans", responses((status = 200, body = PostureScans)), tag = "cloud-security")]
pub async fn scans(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<PostureScans> {
    let accounts: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT id::text, aws_account_id, COALESCE((resources_config->>'cloud_security_posture_management_collection')::boolean, false) \
         FROM aws_accounts WHERE org_id = $1 ORDER BY aws_account_id",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    let latest = store::latest_scans(&st.postgres, &ctx.org_id).await?;
    let accounts = accounts
        .into_iter()
        .map(|(account_ref, account_id, enabled)| {
            let scan = latest.iter().find(|s| s.account_ref == account_ref);
            AccountScan {
                provider: "aws".into(),
                enabled,
                status: scan.map(|s| s.status.clone()),
                queued_at: scan.map(|s| s.queued_at),
                started_at: scan.and_then(|s| s.started_at),
                finished_at: scan.and_then(|s| s.finished_at),
                error: scan.and_then(|s| s.error.clone()),
                tool_version: scan.and_then(|s| s.tool_version.clone()),
                passed: scan.and_then(|s| s.passed),
                failed: scan.and_then(|s| s.failed),
                account_ref,
                account_id,
            }
        })
        .collect();
    Ok(Json(PostureScans { scanner_configured: st.config.scanner_token.is_some(), interval_hours: st.config.posture_scan_interval_hours, accounts }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub account_ref: String,
}

#[utoipa::path(post, path = "/api/v1/cloud-security/scans", operation_id = "cloud_security_scan_now", request_body = ScanRequest,
    responses((status = 204), (status = 400), (status = 404)), tag = "cloud-security")]
pub async fn scan_now(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<ScanRequest>) -> Result<StatusCode, ApiError> {
    if st.config.scanner_token.is_none() {
        return Err(ApiError::BadRequest("No scanner is set up for this install (OC_SCANNER_TOKEN)".into()));
    }
    if !store::scan_now(&st.postgres, &ctx.org_id, &req.account_ref).await? {
        return Err(ApiError::NotFound("No AWS account with Cloud Security scanning turned on".into()));
    }
    ctx.audit(&st, "cloud_security.scan", &req.account_ref, "").await;
    Ok(StatusCode::NO_CONTENT)
}
