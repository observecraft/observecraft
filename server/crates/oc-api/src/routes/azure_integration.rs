//! Integrations → Microsoft Azure: the page's app registrations, the same rows the Datadog-compatible
//! API manages (`datadog_api::azure_integration`, Terraform's `datadog_integration_azure`), and the
//! crawler's progress per subscription.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::datadog_api::azure_integration as dd;
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

/// An app registration: its identity, whether a client secret is stored, and Datadog's other fields.
#[derive(Serialize, ToSchema)]
pub struct AzureAccount {
    pub id: String,
    pub tenant_name: String,
    pub client_id: String,
    pub has_secret: bool,
    #[schema(value_type = Object)]
    pub config: Value,
}

fn view(r: dd::AzureRow) -> AzureAccount {
    AzureAccount { id: r.id, tenant_name: r.tenant_name, client_id: r.client_id, has_secret: r.has_secret, config: r.config }
}

async fn row(st: &AppState, org: &str, id: &str) -> Result<dd::AzureRow, ApiError> {
    dd::get_row(st, org, id).await.map_err(internal)?.ok_or_else(|| ApiError::NotFound("Azure app registration not found".into()))
}

#[utoipa::path(get, path = "/api/v1/integrations/azure/accounts", operation_id = "azure_list_accounts", responses((status = 200, body = [AzureAccount])), tag = "integrations")]
pub async fn list_accounts(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AzureAccount>> {
    Ok(Json(dd::list_rows(&st, &ctx.org_id).await.map_err(internal)?.into_iter().map(view).collect()))
}

/// Datadog's fields (`tenant_name`, `client_id`, `client_secret`, filters, …).
#[utoipa::path(post, path = "/api/v1/integrations/azure/accounts", operation_id = "azure_create_account", request_body = Object, responses((status = 200, body = AzureAccount), (status = 400)), tag = "integrations")]
pub async fn create_account(State(st): State<AppState>, Admin(ctx): Admin, Json(body): Json<Map<String, Value>>) -> ApiResult<AzureAccount> {
    Ok(Json(view(dd::upsert_row(&st, &ctx.org_id, &body).await.map_err(api)?)))
}

/// The fields to change (a new `tenant_name` or `client_id` renames it); the others keep their value.
#[utoipa::path(put, path = "/api/v1/integrations/azure/accounts/{id}", operation_id = "azure_update_account", params(("id" = String, Path)), request_body = Object, responses((status = 200, body = AzureAccount), (status = 400), (status = 404)), tag = "integrations")]
pub async fn update_account(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(body): Json<Map<String, Value>>) -> ApiResult<AzureAccount> {
    let current = row(&st, &ctx.org_id, &id).await?;
    let mut merged = current.config.as_object().cloned().unwrap_or_default();
    merged.extend(body.iter().filter(|(k, _)| *k != "tenant_name" && *k != "client_id").map(|(k, v)| (k.clone(), v.clone())));
    merged.insert("tenant_name".into(), json!(current.tenant_name));
    merged.insert("client_id".into(), json!(current.client_id));
    for (field, new) in [("tenant_name", "new_tenant_name"), ("client_id", "new_client_id")] {
        if let Some(v) = body.get(field) {
            merged.insert(new.into(), v.clone());
        }
    }
    Ok(Json(view(dd::update_row(&st, &ctx.org_id, &merged).await.map_err(api)?)))
}

#[utoipa::path(delete, path = "/api/v1/integrations/azure/accounts/{id}", operation_id = "azure_delete_account", params(("id" = String, Path)), responses((status = 204), (status = 404)), tag = "integrations")]
pub async fn delete_account(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let current = row(&st, &ctx.org_id, &id).await?;
    dd::delete_row(&st, &ctx.org_id, &current.tenant_name, &current.client_id).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct AzureAccountStatus {
    /// One per subscription; scope `""` is the discovery of the subscriptions.
    pub subscriptions: Vec<ScopeStatus>,
}

#[utoipa::path(get, path = "/api/v1/integrations/azure/accounts/{id}/status", operation_id = "azure_account_status", params(("id" = String, Path)), responses((status = 200, body = AzureAccountStatus), (status = 404)), tag = "integrations")]
pub async fn account_status(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<AzureAccountStatus> {
    row(&st, &ctx.org_id, &id).await?;
    Ok(Json(AzureAccountStatus { subscriptions: jobs::status(&st, crate::azure::crawler::PROVIDER, &ctx.org_id, &id).await.map_err(internal)? }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[tokio::test]
    async fn the_page_edits_the_datadog_rows_and_reads_the_crawl_status() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "azure-page").await;
        let Json(created) = create_account(State(st.clone()), Admin(ctx.clone()), Json(body(json!({ "tenant_name": "t", "client_id": "c", "client_secret": "s", "host_filters": "env:prod" })))).await.unwrap();
        assert!(created.has_secret);
        // A partial update keeps the other fields; a new client id renames the registration.
        let Json(updated) = update_account(State(st.clone()), Admin(ctx.clone()), Path(created.id.clone()), Json(body(json!({ "automute": true, "client_id": "c2" })))).await.unwrap();
        assert_eq!((updated.client_id.as_str(), updated.config["host_filters"].as_str(), updated.config["automute"].as_bool(), updated.has_secret), ("c2", Some("env:prod"), Some(true), true));
        let datadog = dd::list_rows(st, &ctx.org_id).await.unwrap();
        assert_eq!((datadog.len(), datadog[0].client_id.as_str()), (1, "c2"));

        sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope, last_success_at) VALUES ('azure', $1, $2::uuid, 'sub-a', now()) ON CONFLICT DO NOTHING")
            .bind(&ctx.org_id)
            .bind(&created.id)
            .execute(&st.postgres)
            .await
            .unwrap();
        let Json(status) = account_status(State(st.clone()), ctx.clone(), Path(created.id.clone())).await.unwrap();
        assert_eq!(status.subscriptions[0].scope, "sub-a");

        let other = crate::test_support::admin_of_new_org(st, "azure-page-other").await;
        assert!(matches!(account_status(State(st.clone()), other, Path(created.id.clone())).await, Err(ApiError::NotFound(_))));
        delete_account(State(st.clone()), Admin(ctx.clone()), Path(created.id)).await.unwrap();
        assert!(dd::list_rows(st, &ctx.org_id).await.unwrap().is_empty());
    }
}
