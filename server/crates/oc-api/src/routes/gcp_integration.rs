//! Integrations → Google Cloud: the page's service accounts, the same rows the Datadog-compatible API
//! manages (`datadog_api::gcp_integration`, Terraform's `datadog_integration_gcp_sts`), this install's
//! service account (the one customers let impersonate theirs), and the crawler's progress per project.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use super::datadog_api::gcp_integration as dd;
use super::datadog_api::DdError;
use crate::auth::{Admin, AuthContext};
use crate::cloud::jobs::{self, ScopeStatus};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

fn api(e: DdError) -> ApiError {
    let msg = e.errors.join(": ");
    match e.status {
        StatusCode::NOT_FOUND => ApiError::NotFound(msg),
        StatusCode::CONFLICT => ApiError::Conflict(msg),
        StatusCode::FORBIDDEN => ApiError::Forbidden(msg),
        StatusCode::BAD_REQUEST => ApiError::BadRequest(msg),
        _ => ApiError::Internal(anyhow::anyhow!(msg)),
    }
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::Internal(e.into())
}

/// A service account: its email and Datadog's other attributes.
#[derive(Serialize, ToSchema)]
pub struct GcpAccount {
    pub id: String,
    pub client_email: String,
    #[schema(value_type = Object)]
    pub config: Value,
}

fn view(r: dd::GcpRow) -> GcpAccount {
    GcpAccount { id: r.id, client_email: r.client_email, config: r.config }
}

#[derive(Serialize, ToSchema)]
pub struct GcpSetup {
    /// This install's service account (`OC_GCP_PRINCIPAL_EMAIL`): customers grant it the Service Account
    /// Token Creator role on their service account. None until it is configured.
    pub principal_email: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/integrations/gcp/setup", operation_id = "gcp_setup", responses((status = 200, body = GcpSetup)), tag = "integrations")]
pub async fn setup(State(st): State<AppState>, _ctx: AuthContext) -> Json<GcpSetup> {
    Json(GcpSetup { principal_email: st.config.gcp_principal_email.clone() })
}

#[utoipa::path(get, path = "/api/v1/integrations/gcp/accounts", operation_id = "gcp_list_accounts", responses((status = 200, body = [GcpAccount])), tag = "integrations")]
pub async fn list_accounts(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<GcpAccount>> {
    Ok(Json(dd::list_rows(&st, &ctx.org_id).await.map_err(internal)?.into_iter().map(view).collect()))
}

/// Datadog's attributes (`client_email`, `account_tags`, `monitored_resource_configs`, …).
#[utoipa::path(post, path = "/api/v1/integrations/gcp/accounts", operation_id = "gcp_create_account", request_body = Object, responses((status = 200, body = GcpAccount), (status = 400), (status = 409)), tag = "integrations")]
pub async fn create_account(State(st): State<AppState>, Admin(ctx): Admin, Json(body): Json<Map<String, Value>>) -> ApiResult<GcpAccount> {
    Ok(Json(view(dd::create_row(&st, &ctx.org_id, &body).await.map_err(api)?)))
}

/// The attributes to change; the others keep their value.
#[utoipa::path(patch, path = "/api/v1/integrations/gcp/accounts/{id}", operation_id = "gcp_update_account", params(("id" = String, Path)), request_body = Object, responses((status = 200, body = GcpAccount), (status = 400), (status = 404)), tag = "integrations")]
pub async fn update_account(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(body): Json<Map<String, Value>>) -> ApiResult<GcpAccount> {
    Ok(Json(view(dd::update_row(&st, &ctx.org_id, &id, &body).await.map_err(api)?)))
}

#[utoipa::path(delete, path = "/api/v1/integrations/gcp/accounts/{id}", operation_id = "gcp_delete_account", params(("id" = String, Path)), responses((status = 204), (status = 404)), tag = "integrations")]
pub async fn delete_account(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    dd::delete_row(&st, &ctx.org_id, &id).await.map_err(api)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct GcpAccountStatus {
    /// One per project; scope `""` is the discovery of the projects.
    pub projects: Vec<ScopeStatus>,
}

#[utoipa::path(get, path = "/api/v1/integrations/gcp/accounts/{id}/status", operation_id = "gcp_account_status", params(("id" = String, Path)), responses((status = 200, body = GcpAccountStatus), (status = 404)), tag = "integrations")]
pub async fn account_status(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<GcpAccountStatus> {
    dd::get_row(&st, &ctx.org_id, &id).await.map_err(internal)?.ok_or_else(|| ApiError::NotFound("GCP service account not found".into()))?;
    Ok(Json(GcpAccountStatus { projects: jobs::status(&st, crate::gcp::crawler::PROVIDER, &ctx.org_id, &id).await.map_err(internal)? }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn the_page_edits_the_datadog_rows_and_reads_the_crawl_status() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "gcp-page").await;
        let body = |v: Value| Json(v.as_object().cloned().unwrap());
        let Json(created) = create_account(State(st.clone()), Admin(ctx.clone()), body(json!({ "client_email": "sa@example-storefront.iam.gserviceaccount.com", "account_tags": ["team:storefront"] }))).await.unwrap();
        let Json(updated) = update_account(State(st.clone()), Admin(ctx.clone()), Path(created.id.clone()), body(json!({ "automute": true }))).await.unwrap();
        assert_eq!((updated.config["automute"].as_bool(), updated.config["account_tags"].clone()), (Some(true), json!(["team:storefront"])));
        sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope, last_success_at) VALUES ('gcp', $1, $2::uuid, 'example-storefront', now()) ON CONFLICT DO NOTHING")
            .bind(&ctx.org_id)
            .bind(&created.id)
            .execute(&st.postgres)
            .await
            .unwrap();
        let Json(status) = account_status(State(st.clone()), ctx.clone(), Path(created.id.clone())).await.unwrap();
        assert_eq!(status.projects[0].scope, "example-storefront");
        let other = crate::test_support::admin_of_new_org(st, "gcp-page-other").await;
        assert!(matches!(account_status(State(st.clone()), other, Path(created.id.clone())).await, Err(ApiError::NotFound(_))));
        delete_account(State(st.clone()), Admin(ctx.clone()), Path(created.id)).await.unwrap();
        let Json(all) = list_accounts(State(st.clone()), ctx).await.unwrap();
        assert!(all.is_empty());
    }
}
