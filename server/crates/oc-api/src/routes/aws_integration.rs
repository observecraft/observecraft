//! Integrations → Amazon Web Services: the page's AWS accounts, the same rows the Datadog-compatible API
//! manages (`datadog_api::aws_integration`, Terraform's `datadog_integration_aws_account`), documented the
//! same way, and what the add-account flows need: the server's AWS principal (`OC_AWS_PRINCIPAL_ARN`,
//! trusted by the customer's role), the IAM permissions, the namespaces and log sources of our
//! catalog, and new external ids. The page builds the CloudFormation template, the Terraform
//! configuration and the manual policies from them.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use super::datadog_api::aws_integration::{self as dd, DdErr};
use crate::auth::{Admin, AuthContext};
use crate::cloud::jobs::{self, ScopeStatus};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

fn api(e: DdErr) -> ApiError {
    let (status, Json(body)) = e;
    let msg = body["errors"][0]["detail"].as_str().unwrap_or_default().to_string();
    match status {
        StatusCode::NOT_FOUND => ApiError::NotFound(msg),
        StatusCode::CONFLICT => ApiError::Conflict(msg),
        StatusCode::FORBIDDEN => ApiError::Forbidden(msg),
        StatusCode::BAD_REQUEST => ApiError::BadRequest(msg),
        _ => ApiError::Internal(anyhow::anyhow!(msg)),
    }
}

/// An account as Datadog's API documents it: `{id, type: "account", attributes}`.
#[derive(Serialize, ToSchema)]
pub struct AwsAccount {
    pub id: String,
    #[schema(value_type = Object)]
    pub attributes: Value,
}

fn view(r: &dd::AccountRow) -> AwsAccount {
    let doc = dd::account_doc(r);
    AwsAccount { id: r.id.clone(), attributes: doc["attributes"].clone() }
}

/// What the add-account flows need.
#[derive(Serialize, ToSchema)]
pub struct AwsSetup {
    /// The AWS principal the server assumes customers' roles with; None until `OC_AWS_PRINCIPAL_ARN` is set.
    pub principal_arn: Option<String>,
    /// IAM actions the integration role must allow.
    pub permissions: Vec<String>,
    /// Namespaces excluded when the account doesn't choose any.
    pub default_excluded_namespaces: Vec<String>,
    /// `[{namespace, name, id, categories}]` (`seeds/aws_integration.json`).
    #[schema(value_type = Object)]
    pub namespaces: Value,
    /// `[{id, name}]`: the log sources the Datadog Forwarder subscribes to.
    #[schema(value_type = Object)]
    pub log_services: Value,
}

pub fn setup_of(principal_arn: Option<String>) -> AwsSetup {
    let seed: Value = serde_json::from_str(include_str!("../../../../seeds/aws_integration.json")).expect("seeds/aws_integration.json is valid");
    AwsSetup {
        principal_arn,
        permissions: dd::STANDARD_PERMISSIONS.iter().map(|p| p.to_string()).collect(),
        default_excluded_namespaces: dd::DEFAULT_EXCLUDED_NAMESPACES.iter().map(|n| n.to_string()).collect(),
        namespaces: seed["namespaces"].clone(),
        log_services: seed["log_services"].clone(),
    }
}

#[derive(Serialize, ToSchema)]
pub struct AwsExternalId {
    pub external_id: String,
}

#[utoipa::path(get, path = "/api/v1/integrations/aws/setup", operation_id = "aws_setup", responses((status = 200, body = AwsSetup)), tag = "integrations")]
pub async fn setup(State(st): State<AppState>, _ctx: AuthContext) -> Json<AwsSetup> {
    Json(setup_of(st.config.aws_principal_arn.clone()))
}

#[utoipa::path(post, path = "/api/v1/integrations/aws/external-id", operation_id = "aws_external_id", responses((status = 200, body = AwsExternalId)), tag = "integrations")]
pub async fn external_id(Admin(_ctx): Admin) -> Json<AwsExternalId> {
    Json(AwsExternalId { external_id: dd::new_external_id() })
}

#[utoipa::path(get, path = "/api/v1/integrations/aws/accounts", operation_id = "aws_list_accounts", responses((status = 200, body = [AwsAccount])), tag = "integrations")]
pub async fn list_accounts(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AwsAccount>> {
    let rows = dd::list_rows(&st, &ctx.org_id, None).await.map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(rows.iter().map(view).collect()))
}

#[utoipa::path(get, path = "/api/v1/integrations/aws/accounts/{id}", operation_id = "aws_get_account", params(("id" = String, Path)), responses((status = 200, body = AwsAccount), (status = 404)), tag = "integrations")]
pub async fn get_account(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<AwsAccount> {
    if !super::scorecards::valid_uuid(&id) {
        return Err(ApiError::NotFound("AWS account not found".into()));
    }
    let row = dd::get_row(&st, &ctx.org_id, &id).await.map_err(|e| ApiError::Internal(e.into()))?;
    row.map(|r| Json(view(&r))).ok_or_else(|| ApiError::NotFound("AWS account not found".into()))
}

/// Datadog's account attributes (`aws_account_id`, `auth_config`, `aws_regions`, `metrics_config`, …).
#[utoipa::path(post, path = "/api/v1/integrations/aws/accounts", operation_id = "aws_create_account", request_body = Object, responses((status = 200, body = AwsAccount), (status = 400), (status = 409)), tag = "integrations")]
pub async fn create_account(State(st): State<AppState>, Admin(ctx): Admin, Json(attributes): Json<Map<String, Value>>) -> ApiResult<AwsAccount> {
    Ok(Json(view(&dd::create_row(&st, &ctx.org_id, &attributes).await.map_err(api)?)))
}

/// The attributes to change; the others keep their value.
#[utoipa::path(patch, path = "/api/v1/integrations/aws/accounts/{id}", operation_id = "aws_update_account", params(("id" = String, Path)), request_body = Object, responses((status = 200, body = AwsAccount), (status = 400), (status = 404)), tag = "integrations")]
pub async fn update_account(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(attributes): Json<Map<String, Value>>) -> ApiResult<AwsAccount> {
    Ok(Json(view(&dd::update_row(&st, &ctx.org_id, &id, &attributes).await.map_err(api)?)))
}

#[utoipa::path(delete, path = "/api/v1/integrations/aws/accounts/{id}", operation_id = "aws_delete_account", params(("id" = String, Path)), responses((status = 204), (status = 404)), tag = "integrations")]
pub async fn delete_account(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    dd::delete_row(&st, &ctx.org_id, &id).await.map_err(api)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct AwsAccountStatus {
    /// One per region.
    pub regions: Vec<ScopeStatus>,
}

/// The crawler's progress and issues per region of an account.
#[utoipa::path(get, path = "/api/v1/integrations/aws/accounts/{id}/status", operation_id = "aws_account_status", params(("id" = String, Path)), responses((status = 200, body = AwsAccountStatus), (status = 404)), tag = "integrations")]
pub async fn account_status(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<AwsAccountStatus> {
    if !super::scorecards::valid_uuid(&id) || dd::get_row(&st, &ctx.org_id, &id).await.map_err(|e| ApiError::Internal(e.into()))?.is_none() {
        return Err(ApiError::NotFound("AWS account not found".into()));
    }
    let regions = jobs::status(&st, crate::aws::crawler::PROVIDER, &ctx.org_id, &id).await.map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(AwsAccountStatus { regions }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn setup_lists_the_catalog_and_the_permissions() {
        let s = setup_of(Some("arn:aws:iam::123456789012:role/observecraft-server".into()));
        assert_eq!(s.principal_arn.as_deref(), Some("arn:aws:iam::123456789012:role/observecraft-server"));
        assert!(s.permissions.iter().any(|p| p == "cloudwatch:GetMetricData"));
        // The lookups of the resources whose ARNs their metrics don't carry (aws/naming.rs LOOKUPS).
        for action in ["apigateway:GET", "rds:DescribeDBProxies", "dms:DescribeReplicationInstances", "waf:ListWebACLs", "waf:GetWebACL", "waf-regional:ListWebACLs", "waf-regional:GetWebACL"] {
            assert!(s.permissions.iter().any(|p| p == action), "{action}");
        }
        assert!(s.namespaces.as_array().unwrap().iter().any(|n| n["namespace"] == "AWS/EC2" && n["name"] == "Amazon EC2"));
        assert!(s.log_services.as_array().unwrap().iter().any(|l| l["id"] == "s3"));
        assert_eq!(s.default_excluded_namespaces, ["AWS/SQS", "AWS/ElasticMapReduce", "AWS/Usage"]);
    }

    #[tokio::test]
    async fn the_status_lists_each_regions_last_run_and_issue() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "aws-status").await;
        let attrs = json!({ "aws_account_id": "123456789012", "auth_config": { "role_name": "R" }, "aws_regions": { "include_only": ["eu-west-1", "us-east-1"] } }).as_object().cloned().unwrap();
        let Json(created) = create_account(State(st.clone()), Admin(ctx.clone()), Json(attrs)).await.unwrap();
        sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope, last_success_at, metrics_last_run, points_last_run) VALUES ('aws', $1, $2::uuid, 'eu-west-1', now(), 12, 340) \
             ON CONFLICT (provider, org_id, account_id, scope) DO UPDATE SET last_success_at = now(), metrics_last_run = 12, points_last_run = 340")
            .bind(&ctx.org_id).bind(&created.id).execute(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope, error_kind, error_message, missing_action, error_at) VALUES ('aws', $1, $2::uuid, 'us-east-1', 'access_denied', 'denied', 'cloudwatch:ListMetrics', now()) \
             ON CONFLICT (provider, org_id, account_id, scope) DO UPDATE SET error_kind = 'access_denied', error_message = 'denied', missing_action = 'cloudwatch:ListMetrics', error_at = now()")
            .bind(&ctx.org_id).bind(&created.id).execute(&st.postgres).await.unwrap();
        let Json(status) = account_status(State(st.clone()), ctx.clone(), Path(created.id.clone())).await.unwrap();
        assert_eq!(status.regions.len(), 2);
        let eu = status.regions.iter().find(|r| r.scope == "eu-west-1").unwrap();
        assert!(eu.last_success_at.is_some() && eu.error.is_none());
        assert_eq!((eu.metrics, eu.points), (12, 340));
        let us = status.regions.iter().find(|r| r.scope == "us-east-1").unwrap();
        let issue = us.error.as_ref().unwrap();
        assert_eq!((issue.kind.as_str(), issue.missing_action.as_deref()), ("access_denied", Some("cloudwatch:ListMetrics")));
        // Another org sees nothing.
        let other = crate::test_support::admin_of_new_org(st, "aws-status-other").await;
        assert!(matches!(account_status(State(st.clone()), other, Path(created.id)).await, Err(ApiError::NotFound(_))));
    }

    #[tokio::test]
    async fn the_page_and_the_datadog_api_share_accounts() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "aws-page").await;
        let attrs = json!({ "aws_account_id": "123456789012", "auth_config": { "role_name": "ObserveCraftIntegrationRole" } }).as_object().cloned().unwrap();
        let Json(created) = create_account(State(st.clone()), Admin(ctx.clone()), Json(attrs.clone())).await.unwrap();
        assert_eq!(created.attributes["auth_config"]["role_name"], "ObserveCraftIntegrationRole");
        assert_eq!(created.attributes["auth_config"]["external_id"].as_str().map(str::len), Some(32));
        assert!(matches!(create_account(State(st.clone()), Admin(ctx.clone()), Json(attrs)).await, Err(ApiError::Conflict(_))));

        // What the page wrote is what the Datadog API answers.
        let dd_rows = dd::list_rows(st, &ctx.org_id, None).await.unwrap();
        assert_eq!(dd_rows.len(), 1);
        assert_eq!(dd_rows[0].id, created.id);

        let change = json!({ "metrics_config": { "enabled": false } }).as_object().cloned().unwrap();
        let Json(updated) = update_account(State(st.clone()), Admin(ctx.clone()), Path(created.id.clone()), Json(change)).await.unwrap();
        assert_eq!(updated.attributes["metrics_config"]["enabled"], false);
        assert_eq!(updated.attributes["aws_account_id"], "123456789012");
        let Json(all) = list_accounts(State(st.clone()), ctx.clone()).await.unwrap();
        assert_eq!(all.len(), 1);

        assert!(matches!(get_account(State(st.clone()), ctx.clone(), Path("nope".into())).await, Err(ApiError::NotFound(_))));
        delete_account(State(st.clone()), Admin(ctx.clone()), Path(created.id.clone())).await.unwrap();
        assert!(matches!(get_account(State(st.clone()), ctx, Path(created.id)).await, Err(ApiError::NotFound(_))));
    }
}
