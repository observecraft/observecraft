//! Cloud Cost → Settings: the providers' bills ObserveCraft reads (`cost_bill_configs`), the same records as
//! Datadog's `aws_cur_config` / `azure_uc_config` (`datadog_api::cost_bills`), with each one's import:
//! when it last read the bills, which months, and its problems.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::datadog_api::cost_bills::{self as dd, AWS, AZURE};
use super::datadog_api::DdError;
use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// A provider's bill config: Datadog's document of it, and its import.
#[derive(Serialize, ToSchema)]
pub struct CostBillConfig {
    pub id: String,
    /// `aws` or `azure`
    pub provider: String,
    /// `aws_cur_config` or `azure_uc_configs`, as Datadog's API answers it.
    #[schema(value_type = Object)]
    pub doc: Value,
    /// The last successful import, epoch ms.
    pub last_import_at: Option<i64>,
    /// The billing months read (Azure's prefixed by their export: `actual:2026-10`).
    pub imported_months: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct EnabledBody {
    pub enabled: bool,
}

fn api(e: DdError) -> ApiError {
    let message = e.errors.join("; ");
    match e.status {
        StatusCode::NOT_FOUND => ApiError::NotFound(message),
        StatusCode::FORBIDDEN => ApiError::Forbidden(message),
        StatusCode::BAD_REQUEST => ApiError::BadRequest(message),
        _ => ApiError::Internal(anyhow::anyhow!(message)),
    }
}

fn provider(p: &str) -> Result<&'static str, ApiError> {
    match p {
        "aws" => Ok(AWS),
        "azure" => Ok(AZURE),
        _ => Err(ApiError::NotFound(format!("no {p} bills"))),
    }
}

fn view(provider: &str, r: &dd::ConfigRow) -> CostBillConfig {
    let doc = if provider == AWS { dd::aws_doc(r) } else { dd::azure_doc(r) };
    let mut imported_months: Vec<String> = r.imported.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    imported_months.sort();
    CostBillConfig { id: r.id.to_string(), provider: provider.to_string(), doc, last_import_at: r.last_import_at, imported_months }
}

async fn one(st: &AppState, org: &str, provider: &'static str, id: &str) -> ApiResult<CostBillConfig> {
    Ok(Json(view(provider, &dd::find_one(st, org, provider, id).await.map_err(api)?)))
}

/// The org's AWS and Azure bill configs with their imports.
#[utoipa::path(get, path = "/api/v1/cloud-cost/bill-configs", operation_id = "cloud_cost_bill_configs", responses((status = 200, body = [CostBillConfig])), tag = "cloud-cost")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<CostBillConfig>> {
    let mut out = vec![];
    for p in [AWS, AZURE] {
        out.extend(dd::find(&st, &ctx.org_id, p, None).await?.iter().map(|r| view(p, r)));
    }
    Ok(Json(out))
}

/// Add an AWS (`aws_cur_config`) or Azure (`azure_uc_config`) bill config: Datadog's create attributes.
#[utoipa::path(post, path = "/api/v1/cloud-cost/bill-configs/{provider}", operation_id = "cloud_cost_create_bill_config", params(("provider" = String, Path)), request_body = Object,
    responses((status = 200, body = CostBillConfig), (status = 400), (status = 404)), tag = "cloud-cost")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Path(p): Path<String>, Json(attrs): Json<Value>) -> ApiResult<CostBillConfig> {
    let p = provider(&p)?;
    let id = if p == AWS { dd::create_aws_config(&st, &ctx.org_id, &attrs).await } else { dd::create_azure_config(&st, &ctx.org_id, &attrs).await }.map_err(api)?;
    one(&st, &ctx.org_id, p, &id.to_string()).await
}

/// Turn a bill config's import on or off.
#[utoipa::path(patch, path = "/api/v1/cloud-cost/bill-configs/{provider}/{id}", operation_id = "cloud_cost_update_bill_config", params(("provider" = String, Path), ("id" = String, Path)), request_body = EnabledBody,
    responses((status = 200, body = CostBillConfig), (status = 404)), tag = "cloud-cost")]
pub async fn set_enabled(State(st): State<AppState>, Admin(ctx): Admin, Path((p, id)): Path<(String, String)>, Json(body): Json<EnabledBody>) -> ApiResult<CostBillConfig> {
    let p = provider(&p)?;
    let row = dd::find_one(&st, &ctx.org_id, p, &id).await.map_err(api)?;
    dd::set_enabled(&st, &ctx.org_id, row.id, body.enabled).await?;
    one(&st, &ctx.org_id, p, &id).await
}

/// Remove (archive) a bill config: its costs stay.
#[utoipa::path(delete, path = "/api/v1/cloud-cost/bill-configs/{provider}/{id}", operation_id = "cloud_cost_delete_bill_config", params(("provider" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 404)), tag = "cloud-cost")]
pub async fn remove(State(st): State<AppState>, Admin(ctx): Admin, Path((p, id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    dd::archive(&st, &ctx.org_id, provider(&p)?, &id).await.map_err(api)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn the_page_adds_lists_disables_and_removes_configs() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "bill-configs").await;
        let cur = json!({ "account_id": "123456789012", "bucket_name": "example-org-cur", "bucket_region": "us-east-1", "report_name": "costs", "report_prefix": "cur" });
        let Json(aws) = create(State(st.clone()), Admin(ctx.clone()), Path("aws".into()), Json(cur.clone())).await.unwrap();
        assert_eq!((aws.provider.as_str(), aws.doc["attributes"]["bucket_name"].as_str()), ("aws", Some("example-org-cur")));
        assert!(matches!(create(State(st.clone()), Admin(ctx.clone()), Path("aws".into()), Json(cur)).await, Err(ApiError::BadRequest(_))));
        let bad_scope = json!({ "account_id": "tenant-1", "client_id": "client-1", "scope": "everything",
            "actual_bill_config": { "export_name": "a", "export_path": "p", "storage_account": "s", "storage_container": "c" },
            "amortized_bill_config": { "export_name": "b", "export_path": "p", "storage_account": "s", "storage_container": "c" } });
        let Err(ApiError::BadRequest(m)) = create(State(st.clone()), Admin(ctx.clone()), Path("azure".into()), Json(bad_scope.clone())).await else { panic!("scope") };
        assert!(m.contains("scope"), "{m}");
        let mut azure = bad_scope;
        azure["scope"] = json!("/subscriptions/sub-1");
        let _ = create(State(st.clone()), Admin(ctx.clone()), Path("azure".into()), Json(azure)).await.unwrap();

        // The import's progress: its last run and the months it read.
        let uid: String = sqlx::query_scalar("SELECT uid::text FROM cost_bill_configs WHERE org_id = $1 AND provider = 'aws'").bind(&ctx.org_id).fetch_one(&st.postgres).await.unwrap();
        sqlx::query("UPDATE cost_bill_configs SET imported = '{\"2026-10\": \"e1\"}' WHERE org_id = $1 AND provider = 'aws'").bind(&ctx.org_id).execute(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope, last_success_at) VALUES ('aws_cur', $1, $2::uuid, '', '2026-11-03T08:00:00Z')").bind(&ctx.org_id).bind(&uid).execute(&st.postgres).await.unwrap();
        let Json(list) = list(State(st.clone()), ctx.clone()).await.unwrap();
        assert_eq!(list.iter().map(|c| c.provider.as_str()).collect::<Vec<_>>(), ["aws", "azure"]);
        assert_eq!((list[0].last_import_at, list[0].imported_months.clone()), (Some(1_793_692_800_000), vec!["2026-10".to_string()]));
        assert_eq!(list[1].doc["attributes"]["configs"][0]["dataset_type"], "amortized");

        let Json(off) = set_enabled(State(st.clone()), Admin(ctx.clone()), Path(("aws".into(), aws.id.clone())), Json(EnabledBody { enabled: false })).await.unwrap();
        assert_eq!(off.doc["attributes"]["status"], "disabled");
        assert_eq!(remove(State(st.clone()), Admin(ctx.clone()), Path(("aws".into(), aws.id.clone()))).await.unwrap(), StatusCode::NO_CONTENT);
        let Json(list) = super::list(State(st.clone()), ctx.clone()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(matches!(remove(State(st.clone()), Admin(ctx), Path(("gcp".into(), "1".into()))).await, Err(ApiError::NotFound(_))));
    }
}
