//! Cloud Cost Management's provider bills, Datadog-compatible: where an AWS Cost and Usage Report
//! (`/api/v2/cost/aws_cur_config`, the provider's `datadog_aws_cur_config`) and Azure's actual and
//! amortized cost exports (`/api/v2/cost/azure_uc_config`, `datadog_azure_uc_config`) are delivered.
//!
//! - Ids are Datadog's numeric cloud account ids (as strings). An Azure pair (its two exports) is
//!   one id; its `configs` list the amortized export, then the actual one.
//! - AWS `PATCH` sets `is_enabled` or `account_filters` and answers a list, Azure's `is_enabled`
//!   and answers the pair. `DELETE` archives (204): the config leaves the API and stops importing,
//!   its costs stay.
//! - `error_messages` are the last import's problems.
//!
//! Tenancy: every query filters on `org_id`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::users::{bad_request, body, data, ok, require, DocResult};
use super::{DdAuth, DdError};
use crate::auth::Access;
use crate::state::AppState;

pub const AWS: &str = "aws";
pub const AZURE: &str = "azure";
/// Months of bills imported, as Datadog's default.
const DEFAULT_MONTHS: i32 = 15;
const MAX_MONTHS: i32 = 36;

#[derive(sqlx::FromRow)]
pub struct ConfigRow {
    pub id: i64,
    pub account_id: String,
    pub scope: String,
    pub config: Value,
    pub months: i32,
    pub status: String,
    status_updated_at: String,
    created_at: String,
    updated_at: String,
    errors: Vec<String>,
    /// The import's last successful run, epoch ms.
    pub last_import_at: Option<i64>,
    /// Billing months read (`YYYY-MM`, Azure `actual:YYYY-MM`) → the run read.
    pub imported: Value,
}

/// Datadog's timestamps: microseconds, no zone.
fn ts(column: &str) -> String {
    format!("to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US') AS {column}")
}

fn cols() -> String {
    format!(
        "c.id, c.account_id, c.scope, c.config, c.months, c.status, {}, {}, {}, \
         COALESCE((SELECT array_agg(j.error_message ORDER BY j.scope) FROM cloud_crawl_jobs j WHERE j.org_id = c.org_id AND j.account_id = c.uid AND j.error_message IS NOT NULL), '{{}}') AS errors, \
         (SELECT (extract(epoch FROM max(j.last_success_at)) * 1000)::bigint FROM cloud_crawl_jobs j WHERE j.org_id = c.org_id AND j.account_id = c.uid) AS last_import_at, c.imported",
        ts("status_updated_at"),
        ts("created_at"),
        ts("updated_at")
    )
}

/// The org's configs of a provider (archived ones aside), or the one `id` names.
pub async fn find(st: &AppState, org: &str, provider: &str, id: Option<i64>) -> Result<Vec<ConfigRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM cost_bill_configs c WHERE c.org_id = $1 AND c.provider = $2 AND c.status <> 'archived' AND ($3::bigint IS NULL OR c.id = $3) ORDER BY c.id",
        cols()
    )))
    .bind(org)
    .bind(provider)
    .bind(id)
    .fetch_all(&st.postgres)
    .await
}

fn not_found(id: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, format!("Cloud account with ID {id} was not found"))
}

pub async fn find_one(st: &AppState, org: &str, provider: &str, id: &str) -> Result<ConfigRow, DdError> {
    let n = id.parse::<i64>().map_err(|_| not_found(id))?;
    find(st, org, provider, Some(n)).await?.into_iter().next().ok_or_else(|| not_found(id))
}

fn text(attrs: &Value, key: &str) -> Option<String> {
    attrs.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn required(attrs: &Value, key: &str) -> Result<String, DdError> {
    text(attrs, key).ok_or_else(|| bad_request(format!("attribute \"{key}\" is required")))
}

fn months(attrs: &Value) -> Result<i32, DdError> {
    match attrs.get("months") {
        None | Some(Value::Null) => Ok(DEFAULT_MONTHS),
        Some(v) => v.as_i64().filter(|m| (1..=MAX_MONTHS as i64).contains(m)).map(|m| m as i32).ok_or_else(|| bad_request(format!("attribute \"months\" must be between 1 and {MAX_MONTHS}"))),
    }
}

/// `account_filters`, normalized; None when it filters nothing (`{}` clears them).
fn account_filters(v: &Value) -> Result<Option<Value>, DdError> {
    let Some(m) = v.as_object() else { return if v.is_null() { Ok(None) } else { Err(bad_request("attribute \"account_filters\" must be an object")) } };
    let list = |key: &str| -> Result<Vec<String>, DdError> {
        match m.get(key) {
            None | Some(Value::Null) => Ok(vec![]),
            Some(Value::Array(items)) => items.iter().map(|i| i.as_str().map(str::to_string).ok_or_else(|| bad_request(format!("attribute \"{key}\" must be a list of account ids")))).collect(),
            Some(_) => Err(bad_request(format!("attribute \"{key}\" must be a list of account ids"))),
        }
    };
    let include_new = m.get("include_new_accounts").and_then(Value::as_bool);
    let (included, excluded) = (list("included_accounts")?, list("excluded_accounts")?);
    if include_new.is_none() && included.is_empty() && excluded.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({ "include_new_accounts": include_new, "included_accounts": included, "excluded_accounts": excluded })))
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505")
}

async fn insert(st: &AppState, org: &str, provider: &str, account_id: &str, scope: &str, config: &Value, months: i32) -> Result<i64, DdError> {
    let r: Result<i64, sqlx::Error> = sqlx::query_scalar("INSERT INTO cost_bill_configs (org_id, provider, account_id, scope, config, months) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id")
        .bind(org)
        .bind(provider)
        .bind(account_id)
        .bind(scope)
        .bind(config)
        .bind(months)
        .fetch_one(&st.postgres)
        .await;
    match r {
        Ok(id) => Ok(id),
        Err(e) if is_unique_violation(&e) => Err(bad_request(format!("Cloud account {account_id} is already configured"))),
        Err(e) => Err(e.into()),
    }
}

pub async fn set_enabled(st: &AppState, org: &str, id: i64, enabled: bool) -> Result<(), sqlx::Error> {
    let status = if enabled { "active" } else { "disabled" };
    sqlx::query("UPDATE cost_bill_configs SET status = $3, status_updated_at = CASE WHEN status = $3 THEN status_updated_at ELSE now() END, updated_at = now() WHERE org_id = $1 AND id = $2")
        .bind(org)
        .bind(id)
        .bind(status)
        .execute(&st.postgres)
        .await?;
    Ok(())
}

pub async fn archive(st: &AppState, org: &str, provider: &str, id: &str) -> Result<StatusCode, DdError> {
    let row = find_one(st, org, provider, id).await?;
    sqlx::query("UPDATE cost_bill_configs SET status = 'archived', status_updated_at = now(), updated_at = now() WHERE org_id = $1 AND id = $2").bind(org).bind(row.id).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn common(r: &ConfigRow) -> Value {
    json!({
        "account_id": r.account_id, "months": r.months, "status": r.status, "status_updated_at": r.status_updated_at,
        "created_at": r.created_at, "updated_at": r.updated_at, "error_messages": r.errors,
    })
}

// ================================================================ AWS

pub fn aws_doc(r: &ConfigRow) -> Value {
    let mut a = common(r);
    for key in ["bucket_name", "bucket_region", "report_name", "report_prefix"] {
        a[key] = r.config[key].clone();
    }
    a["account_filters"] = match &r.config["account_filters"] {
        Value::Null => json!({ "include_new_accounts": null }),
        f => f.clone(),
    };
    json!({ "type": "aws_cur_config", "id": r.id.to_string(), "attributes": a })
}

/// List the AWS CUR configs (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/cost/aws_cur_config", operation_id = "datadog_list_cost_aws_cur_configs", responses((status = 200, body = super::users::DatadogIamDocument)), tag = "datadog-api")]
pub async fn list_aws(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows = find(&st, &ctx.org_id, AWS, None).await?;
    ok(json!({ "data": rows.iter().map(aws_doc).collect::<Vec<_>>() }))
}

/// Get an AWS CUR config (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/cost/aws_cur_config/{cloud_account_id}", operation_id = "datadog_get_cost_aws_cur_config", params(("cloud_account_id" = String, Path)),
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_aws(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": aws_doc(&find_one(&st, &ctx.org_id, AWS, &id).await?) }))
}

/// Create an AWS CUR config (Datadog-compatible): where the account's Cost and Usage Report is delivered.
#[utoipa::path(post, path = "/api/v2/cost/aws_cur_config", operation_id = "datadog_create_cost_aws_cur_config", request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_aws(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let id = create_aws_config(&st, &ctx.org_id, &data(&doc)?["attributes"]).await?;
    ok(json!({ "data": aws_doc(&find_one(&st, &ctx.org_id, AWS, &id.to_string()).await?) }))
}

/// Validates and stores an AWS CUR config from its attributes (Datadog's create request).
pub async fn create_aws_config(st: &AppState, org: &str, attrs: &Value) -> Result<i64, DdError> {
    let account_id = required(attrs, "account_id")?;
    if account_id.len() != 12 || !account_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad_request("attribute \"account_id\" must be a 12-digit AWS account id"));
    }
    let prefix = required(attrs, "report_prefix")?;
    if prefix.starts_with('/') || prefix.ends_with('/') {
        return Err(bad_request("attribute \"report_prefix\" can't start or end with \"/\""));
    }
    let config = json!({
        "bucket_name": required(attrs, "bucket_name")?,
        "bucket_region": text(attrs, "bucket_region").unwrap_or_else(|| "us-east-1".into()),
        "report_name": required(attrs, "report_name")?,
        "report_prefix": prefix,
        "account_filters": account_filters(attrs.get("account_filters").unwrap_or(&Value::Null))?,
    });
    insert(st, org, AWS, &account_id, "", &config, months(attrs)?).await
}

/// Update an AWS CUR config (Datadog-compatible): `is_enabled`, `account_filters`; answers a list.
#[utoipa::path(patch, path = "/api/v2/cost/aws_cur_config/{cloud_account_id}", operation_id = "datadog_update_cost_aws_cur_config", params(("cloud_account_id" = String, Path)), request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_aws(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let row = find_one(&st, &ctx.org_id, AWS, &id).await?;
    let doc = body(&raw)?;
    let attrs = &data(&doc)?["attributes"];
    if let Some(filters) = attrs.get("account_filters") {
        sqlx::query("UPDATE cost_bill_configs SET config = jsonb_set(config, '{account_filters}', $3), updated_at = now() WHERE org_id = $1 AND id = $2")
            .bind(&ctx.org_id)
            .bind(row.id)
            .bind(account_filters(filters)?.unwrap_or(Value::Null))
            .execute(&st.postgres)
            .await?;
    }
    if let Some(enabled) = attrs.get("is_enabled").and_then(Value::as_bool) {
        set_enabled(&st, &ctx.org_id, row.id, enabled).await?;
    }
    ok(json!({ "data": [aws_doc(&find_one(&st, &ctx.org_id, AWS, &id).await?)] }))
}

/// Delete (archive) an AWS CUR config (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/cost/aws_cur_config/{cloud_account_id}", operation_id = "datadog_delete_cost_aws_cur_config", params(("cloud_account_id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_aws(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    archive(&st, &ctx.org_id, AWS, &id).await
}

// ================================================================ Azure

/// The scopes Azure cost exports are made at: a subscription, a resource group, a billing account (or
/// its billing profile) or a management group, with or without the leading `/`.
pub fn valid_scope(scope: &str) -> bool {
    let parts: Vec<&str> = scope.strip_prefix('/').unwrap_or(scope).split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    let eq = |p: &str, s: &str| p.eq_ignore_ascii_case(s);
    match parts.as_slice() {
        [s, _] => eq(s, "subscriptions"),
        [s, _, g, _] if eq(s, "subscriptions") => eq(g, "resourceGroups"),
        [p, ns, kind, _] => eq(p, "providers") && ((eq(ns, "Microsoft.Billing") && eq(kind, "billingAccounts")) || (eq(ns, "Microsoft.Management") && eq(kind, "managementGroups"))),
        [p, ns, a, _, b, _] => eq(p, "providers") && eq(ns, "Microsoft.Billing") && eq(a, "billingAccounts") && eq(b, "billingProfiles"),
        _ => false,
    }
}

/// An Azure pair's two exports.
pub const DATASETS: [&str; 2] = ["amortized", "actual"];

pub fn azure_doc(r: &ConfigRow) -> Value {
    let configs: Vec<Value> = DATASETS
        .iter()
        .map(|dataset| {
            let mut a = common(r);
            a["id"] = json!(r.id.to_string());
            a["client_id"] = r.config["client_id"].clone();
            a["scope"] = json!(r.scope);
            a["dataset_type"] = json!(dataset);
            for key in ["export_name", "export_path", "storage_account", "storage_container"] {
                a[key] = r.config[*dataset][key].clone();
            }
            a
        })
        .collect();
    json!({ "type": "azure_uc_configs", "id": r.id.to_string(), "attributes": { "configs": configs } })
}

fn bill_config(attrs: &Value, key: &str) -> Result<Value, DdError> {
    let c = attrs.get(key).filter(|c| c.is_object()).ok_or_else(|| bad_request(format!("attribute \"{key}\" is required")))?;
    let field = |f: &str| required(c, f).map_err(|_| bad_request(format!("attribute \"{key}.{f}\" is required")));
    Ok(json!({ "export_name": field("export_name")?, "export_path": field("export_path")?, "storage_account": field("storage_account")?, "storage_container": field("storage_container")? }))
}

/// List the Azure UC configs (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/cost/azure_uc_config", operation_id = "datadog_list_cost_azure_uc_configs", responses((status = 200, body = super::users::DatadogIamDocument)), tag = "datadog-api")]
pub async fn list_azure(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows = find(&st, &ctx.org_id, AZURE, None).await?;
    ok(json!({ "data": rows.iter().map(azure_doc).collect::<Vec<_>>() }))
}

/// Get an Azure UC config pair (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/cost/azure_uc_config/{cloud_account_id}", operation_id = "datadog_get_cost_azure_uc_config", params(("cloud_account_id" = String, Path)),
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_azure(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": azure_doc(&find_one(&st, &ctx.org_id, AZURE, &id).await?) }))
}

/// Create an Azure UC config pair (Datadog-compatible): the actual and amortized cost exports of a scope.
#[utoipa::path(post, path = "/api/v2/cost/azure_uc_config", operation_id = "datadog_create_cost_azure_uc_configs", request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_azure(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let doc = body(&raw)?;
    let id = create_azure_config(&st, &ctx.org_id, &data(&doc)?["attributes"]).await?;
    ok(json!({ "data": azure_doc(&find_one(&st, &ctx.org_id, AZURE, &id.to_string()).await?) }))
}

/// Validates and stores an Azure UC config pair from its attributes (Datadog's create request).
pub async fn create_azure_config(st: &AppState, org: &str, attrs: &Value) -> Result<i64, DdError> {
    let account_id = required(attrs, "account_id")?;
    let client_id = required(attrs, "client_id")?;
    let scope = required(attrs, "scope")?;
    if !valid_scope(&scope) {
        return Err(bad_request("attribute \"scope\" does not match the required format"));
    }
    let config = json!({ "client_id": client_id, "actual": bill_config(attrs, "actual_bill_config")?, "amortized": bill_config(attrs, "amortized_bill_config")? });
    insert(st, org, AZURE, &account_id, &scope, &config, months(attrs)?).await
}

/// Enable or disable an Azure UC config pair (Datadog-compatible).
#[utoipa::path(patch, path = "/api/v2/cost/azure_uc_config/{cloud_account_id}", operation_id = "datadog_update_cost_azure_uc_configs", params(("cloud_account_id" = String, Path)), request_body = Value,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_azure(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let row = find_one(&st, &ctx.org_id, AZURE, &id).await?;
    let doc = body(&raw)?;
    let enabled = data(&doc)?["attributes"].get("is_enabled").and_then(Value::as_bool).ok_or_else(|| bad_request("attribute \"is_enabled\" is required"))?;
    set_enabled(&st, &ctx.org_id, row.id, enabled).await?;
    ok(json!({ "data": azure_doc(&find_one(&st, &ctx.org_id, AZURE, &id).await?) }))
}

/// Delete (archive) an Azure UC config pair (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/cost/azure_uc_config/{cloud_account_id}", operation_id = "datadog_delete_cost_azure_uc_config", params(("cloud_account_id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_azure(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    archive(&st, &ctx.org_id, AZURE, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::datadog_api::replay::{Feature, Replayer, TestServer};

    async fn replay_named(r: &mut Replayer, f: &Feature, names: &[&str]) -> Vec<String> {
        let mut problems = vec![];
        for name in names {
            let interactions = f.cassette_named(name).unwrap_or_else(|| panic!("no cassette for {name:?}"));
            let (_, p) = r.replay_interactions(f.sends(f.scenario(name)), name, &interactions).await;
            problems.extend(p);
        }
        problems
    }

    #[tokio::test]
    async fn replays_datadogs_cur_and_uc_config_scenarios() {
        let Some(f) = Feature::load("v2/cloud_cost_management") else { return };
        let Some(s) = TestServer::start("dd-cost-bills").await else { return };
        let mut problems = vec![];
        // Unknown ids first: nothing exists yet.
        let mut r = s.replayer();
        problems.extend(
            replay_named(
                &mut r,
                &f,
                &[
                    "Delete Cloud Cost Management AWS CUR config returns \"Not Found\" response",
                    "Update Cloud Cost Management AWS CUR config returns \"Not Found\" response",
                    "Delete Cloud Cost Management Azure config returns \"Not Found\" response",
                    "Update Cloud Cost Management Azure config returns \"Not Found\" response",
                    "Create Cloud Cost Management Azure configs returns \"Bad Request\" response",
                    "Create Cloud Cost Management AWS CUR config returns \"OK\" response",
                    "Create Cloud Cost Management Azure configs returns \"OK\" response",
                ],
            )
            .await,
        );
        let http = reqwest::Client::new();
        let get = |path: &str| http.get(format!("{}{path}", s.base)).header("dd-application-key", &s.app_key).header("dd-api-key", &s.api_key).send();
        let aws: Value = get("/api/v2/cost/aws_cur_config").await.unwrap().json().await.unwrap();
        let azure: Value = get("/api/v2/cost/azure_uc_config").await.unwrap().json().await.unwrap();
        let aws_id = aws["data"][0]["id"].clone();
        let azure_id = azure["data"][0]["id"].clone();
        assert!(aws_id.as_str().is_some_and(|i| i.parse::<i64>().is_ok()), "{aws}");
        assert!(azure_id.as_str().is_some_and(|i| i.parse::<i64>().is_ok()), "{azure}");
        assert_eq!(azure["data"][0]["attributes"]["configs"][0]["dataset_type"], "amortized", "{azure}");

        // The recordings of fixed upstream ids, against ours.
        let mut r = s.replayer();
        for recorded in ["100", "123456"] {
            r.map_id(recorded, aws_id.clone());
        }
        problems.extend(
            replay_named(
                &mut r,
                &f,
                &[
                    "Get cost AWS CUR config returns \"OK\" response",
                    "List Cloud Cost Management AWS CUR configs returns \"OK\" response",
                    "Update Cloud Cost Management AWS CUR config returns \"OK\" response",
                    "Delete Cloud Cost Management AWS CUR config returns \"No Content\" response",
                ],
            )
            .await,
        );
        let mut r = s.replayer();
        for recorded in ["100", "123456"] {
            r.map_id(recorded, azure_id.clone());
        }
        problems.extend(
            replay_named(
                &mut r,
                &f,
                &[
                    "Get cost Azure UC config returns \"OK\" response",
                    "List Cloud Cost Management Azure configs returns \"OK\" response",
                    "Update Cloud Cost Management Azure config returns \"OK\" response",
                    "Delete Cloud Cost Management Azure config returns \"No Content\" response",
                ],
            )
            .await,
        );
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        // Deleted configs are archived: gone from the API.
        assert_eq!(get(&format!("/api/v2/cost/aws_cur_config/{}", aws_id.as_str().unwrap())).await.unwrap().status(), 404);
        assert_eq!(get("/api/v2/cost/azure_uc_config").await.unwrap().json::<Value>().await.unwrap()["data"], json!([]));
    }

    #[tokio::test]
    async fn configs_are_one_per_account_and_patch_what_terraform_sends() {
        let Some(s) = TestServer::start("dd-cost-bills-rules").await else { return };
        let http = reqwest::Client::new();
        let send = |method: reqwest::Method, path: &str, body: Option<Value>| {
            let req = http.request(method, format!("{}{path}", s.base)).header("dd-application-key", &s.app_key).header("dd-api-key", &s.api_key);
            match body {
                Some(b) => req.json(&b),
                None => req,
            }
            .send()
        };
        let cur = json!({ "data": { "type": "aws_cur_config_post_request", "attributes": {
            "account_id": "123456789012", "bucket_name": "example-cur", "bucket_region": "us-east-1", "report_name": "costs", "report_prefix": "cur" } } });
        let res = send(reqwest::Method::POST, "/api/v2/cost/aws_cur_config", Some(cur.clone())).await.unwrap();
        assert_eq!(res.status(), 200);
        let created: Value = res.json().await.unwrap();
        let id = created["data"]["id"].as_str().unwrap().to_string();
        let a = &created["data"]["attributes"];
        assert_eq!((a["status"].as_str(), a["months"].as_i64(), a["error_messages"].clone()), (Some("active"), Some(15), json!([])));
        let ts = a["created_at"].as_str().unwrap();
        assert!(ts.len() == 26 && ts.as_bytes()[19] == b'.' && !ts.ends_with('Z'), "{ts}");

        // One config per account.
        let res = send(reqwest::Method::POST, "/api/v2/cost/aws_cur_config", Some(cur.clone())).await.unwrap();
        assert_eq!(res.status(), 400);
        // Required fields and Datadog's prefix rule.
        let mut bad = cur.clone();
        bad["data"]["attributes"]["account_id"] = json!("210987654321");
        bad["data"]["attributes"]["report_prefix"] = json!("/cur/");
        assert_eq!(send(reqwest::Method::POST, "/api/v2/cost/aws_cur_config", Some(bad)).await.unwrap().status(), 400);

        // Terraform's update: account filters only; `is_enabled` disables.
        let filters = json!({ "include_new_accounts": false, "included_accounts": ["123456789012"], "excluded_accounts": [] });
        let patch = json!({ "data": { "type": "aws_cur_config_patch_request", "attributes": { "account_filters": filters } } });
        let res: Value = send(reqwest::Method::PATCH, &format!("/api/v2/cost/aws_cur_config/{id}"), Some(patch)).await.unwrap().json().await.unwrap();
        assert_eq!(res["data"][0]["attributes"]["account_filters"], filters, "{res}");
        let off = json!({ "data": { "type": "aws_cur_config_patch_request", "attributes": { "is_enabled": false } } });
        let res: Value = send(reqwest::Method::PATCH, &format!("/api/v2/cost/aws_cur_config/{id}"), Some(off)).await.unwrap().json().await.unwrap();
        assert_eq!(res["data"][0]["attributes"]["status"], "disabled");
        assert_eq!(res["data"][0]["attributes"]["account_filters"], filters, "filters are kept: {res}");

        // Archived, the account can be configured again.
        assert_eq!(send(reqwest::Method::DELETE, &format!("/api/v2/cost/aws_cur_config/{id}"), None).await.unwrap().status(), 204);
        assert_eq!(send(reqwest::Method::POST, "/api/v2/cost/aws_cur_config", Some(cur)).await.unwrap().status(), 200);
        // Another org doesn't see it.
        let other = crate::routes::datadog_api::replay::new_org_with_roles(&s.st, "dd-cost-bills-other").await;
        let (_, other_key) = crate::routes::datadog_api::replay::user_with_app_key(&s.st, &other, "admin").await;
        let other_api = crate::routes::datadog_api::replay::ingest_key(&s.st, &other).await;
        let theirs: Value = http.get(format!("{}/api/v2/cost/aws_cur_config", s.base)).header("dd-application-key", &other_key).header("dd-api-key", &other_api).send().await.unwrap().json().await.unwrap();
        assert_eq!(theirs["data"], json!([]));
    }

    #[test]
    fn azure_scopes_are_the_exports_scopes() {
        for ok in [
            "subscriptions/1234abcd-1234-abcd-1234-1234abcd1234",
            "/subscriptions/1234abcd-1234-abcd-1234-1234abcd1234/resourceGroups/storefront",
            "/providers/Microsoft.Billing/billingAccounts/12345678",
            "providers/Microsoft.Billing/billingAccounts/1234:5678_2019-05-31/billingProfiles/ABCD-EFGH",
            "/providers/Microsoft.Management/managementGroups/example-org",
        ] {
            assert!(valid_scope(ok), "{ok}");
        }
        for bad in ["this_is_an_invalid_scope", "", "/subscriptions/", "/subscriptions/a/b", "subscriptions/a/resourceGroups/"] {
            assert!(!valid_scope(bad), "{bad}");
        }
    }
}
