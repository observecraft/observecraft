//! The GCP integration, Datadog-compatible: STS service accounts (`/api/v2/integration/gcp/accounts`,
//! the Terraform provider's `datadog_integration_gcp_sts`) on Postgres `gcp_accounts`, and the STS
//! delegate (`/api/v2/integration/gcp/sts_delegate`): this install's own service account.

//!
//! - Every attribute the provider reads back is answered, with defaults when it wasn't sent; an update
//!   keeps the attributes it doesn't send. CSPM needs resource collection (400, as Datadog).
//! - No key is stored: this install impersonates the customer's service account with its own
//!   (`OC_GCP_PRINCIPAL_EMAIL`), which the customer lets create tokens for it. The delegate endpoints
//!   answer that account (404 until it is configured).
//! - A list's `meta.accessible_projects` are the projects the crawler found.
//! - Writes need an admin. Errors are `{"errors": ["…"]}`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};

use super::users::require;
use super::{DdAuth, DdError};
use crate::auth::Access;
use crate::routes::scorecards::valid_uuid;
use crate::state::AppState;

fn bad(msg: &str) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, msg)
}

const BOOLS: [(&str, bool); 8] = [
    ("automute", false),
    ("is_cspm_enabled", false),
    ("is_security_command_center_enabled", false),
    ("is_resource_change_collection_enabled", false),
    ("is_per_project_quota_enabled", false),
    ("resource_collection_enabled", false),
    ("is_org_folder_resource_collection_enabled", false),
    ("is_global_location_enabled", true),
];
const STRING_LISTS: [&str; 4] = ["account_tags", "host_filters", "cloud_run_revision_filters", "region_filter_configs"];

fn strings(v: &Value, key: &str) -> Result<Value, DdError> {
    match v {
        Value::Null => Ok(json!([])),
        Value::Array(items) if items.iter().all(Value::is_string) => Ok(v.clone()),
        _ => Err(bad(&format!("{key} must be a list of strings"))),
    }
}

fn configs(v: &Value, key: &str, id_key: &str) -> Result<Value, DdError> {
    let mut out = vec![];
    for c in v.as_array().map(Vec::as_slice).unwrap_or_default() {
        let id = c.get(id_key).and_then(Value::as_str).ok_or_else(|| bad(&format!("{key}.{id_key} is required")))?;
        let mut item = json!({ id_key: id, "filters": strings(c.get("filters").unwrap_or(&Value::Null), &format!("{key}.filters"))? });
        if let Some(d) = c.get("disabled").and_then(Value::as_bool) {
            item["disabled"] = json!(d);
        }
        out.push(item);
    }
    Ok(Value::Array(out))
}

/// Checks a create (`current` None) or an update (`current`: `{client_email, config}`, whose attributes
/// it keeps unless sent): the client email and the normalized attributes.
pub fn gcp_input(attrs: &Map<String, Value>, current: Option<&Value>) -> Result<(String, Value), DdError> {
    let email = attrs.get("client_email").and_then(Value::as_str).map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).or_else(|| current.and_then(|c| c["client_email"].as_str()).map(str::to_string));
    let Some(email) = email else { return Err(bad("client_email is required")) };
    let kept = current.map(|c| c["config"].clone()).unwrap_or_default();
    let get = |key: &str| attrs.get(key).filter(|v| !v.is_null()).or_else(|| kept.get(key));
    let mut config = Map::new();
    for (key, default) in BOOLS {
        let v = match get(key) {
            None => default,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(bad(&format!("{key} must be a boolean"))),
        };
        config.insert(key.into(), json!(v));
    }
    for key in STRING_LISTS {
        config.insert(key.into(), strings(get(key).unwrap_or(&Value::Null), key)?);
    }
    config.insert("metric_namespace_configs".into(), configs(get("metric_namespace_configs").unwrap_or(&Value::Null), "metric_namespace_configs", "id")?);
    config.insert("monitored_resource_configs".into(), configs(get("monitored_resource_configs").unwrap_or(&Value::Null), "monitored_resource_configs", "type")?);
    if config["is_cspm_enabled"] == json!(true) && config["resource_collection_enabled"] == json!(false) {
        return Err(bad("Resource Collection must be enabled for CSM to be enabled"));
    }
    Ok((email, Value::Object(config)))
}

/// A service account as Datadog's API documents it.
pub fn gcp_doc(id: &str, client_email: &str, config: &Value) -> Value {
    let mut attributes = config.as_object().cloned().unwrap_or_default();
    attributes.insert("client_email".into(), json!(client_email));
    json!({ "id": id, "type": "gcp_service_account", "attributes": attributes })
}

// ================================================================ storage

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct GcpRow {
    pub id: String,
    pub client_email: String,
    pub config: Value,
}

const SELECT: &str = "SELECT id::text AS id, client_email, config FROM gcp_accounts";

fn db(e: sqlx::Error) -> DdError {
    tracing::error!(error = %e, "GCP integration request failed");
    DdError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn not_found() -> DdError {
    DdError::new(StatusCode::NOT_FOUND, "GCP service account not found")
}

pub async fn list_rows(st: &AppState, org: &str) -> Result<Vec<GcpRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE org_id = $1 ORDER BY created_at, id"))).bind(org).fetch_all(&st.postgres).await
}

pub async fn get_row(st: &AppState, org: &str, id: &str) -> Result<Option<GcpRow>, sqlx::Error> {
    if !valid_uuid(id) {
        return Ok(None);
    }
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE org_id = $1 AND id = $2::uuid"))).bind(org).bind(id).fetch_optional(&st.postgres).await
}

fn conflict_or(e: sqlx::Error) -> DdError {
    if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") {
        return DdError::new(StatusCode::CONFLICT, "a GCP service account with this client email already exists");
    }
    db(e)
}

pub async fn create_row(st: &AppState, org: &str, attrs: &Map<String, Value>) -> Result<GcpRow, DdError> {
    let _changed = crate::routes::dashboards::EnabledChange(org);
    let (email, config) = gcp_input(attrs, None)?;
    sqlx::query_as("INSERT INTO gcp_accounts (org_id, client_email, config) VALUES ($1, $2, $3) RETURNING id::text AS id, client_email, config")
        .bind(org)
        .bind(&email)
        .bind(&config)
        .fetch_one(&st.postgres)
        .await
        .map_err(conflict_or)
}

pub async fn update_row(st: &AppState, org: &str, id: &str, attrs: &Map<String, Value>) -> Result<GcpRow, DdError> {
    let current = get_row(st, org, id).await.map_err(db)?.ok_or_else(not_found)?;
    let (email, config) = gcp_input(attrs, Some(&json!({ "client_email": current.client_email, "config": current.config })))?;
    sqlx::query_as("UPDATE gcp_accounts SET client_email = $3, config = $4, modified_at = now() WHERE org_id = $1 AND id = $2::uuid RETURNING id::text AS id, client_email, config")
        .bind(org)
        .bind(id)
        .bind(&email)
        .bind(&config)
        .fetch_one(&st.postgres)
        .await
        .map_err(conflict_or)
}

pub async fn delete_row(st: &AppState, org: &str, id: &str) -> Result<(), DdError> {
    let _changed = crate::routes::dashboards::EnabledChange(org);
    if !valid_uuid(id) {
        return Err(bad("invalid account id"));
    }
    let n = sqlx::query("DELETE FROM gcp_accounts WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).execute(&st.postgres).await.map_err(db)?.rows_affected();
    if n == 0 {
        return Err(not_found());
    }
    Ok(())
}

// ================================================================ handlers

/// `data.attributes` of a JSON:API body of type `gcp_service_account`.
fn body_attrs(raw: &Bytes) -> Result<Map<String, Value>, DdError> {
    let v: Value = serde_json::from_slice(raw).map_err(|_| bad("Invalid JSON body"))?;
    let d = v.get("data").and_then(Value::as_object).ok_or_else(|| bad("data is required"))?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != "gcp_service_account") {
        return Err(bad("data.type must be gcp_service_account"));
    }
    Ok(d.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default())
}

#[utoipa::path(get, path = "/api/v2/integration/gcp/accounts", operation_id = "datadog_list_gcp_sts_accounts", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> Result<Json<Value>, DdError> {
    let rows = list_rows(&st, &ctx.org_id).await.map_err(db)?;
    let mut data = vec![];
    for r in rows {
        let projects: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = 'gcp' AND org_id = $1 AND account_id = $2::uuid AND scope <> '' ORDER BY scope")
            .bind(&ctx.org_id)
            .bind(&r.id)
            .fetch_all(&st.postgres)
            .await
            .map_err(db)?;
        let mut doc = gcp_doc(&r.id, &r.client_email, &r.config);
        doc["meta"] = json!({ "accessible_projects": projects });
        data.push(doc);
    }
    Ok(Json(json!({ "data": data })))
}

#[utoipa::path(post, path = "/api/v2/integration/gcp/accounts", operation_id = "datadog_create_gcp_sts_account", request_body = Object, responses((status = 201, body = Object), (status = 400, body = Object), (status = 409, body = Object)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> Result<(StatusCode, Json<Value>), DdError> {
    require(&ctx, Access::Admin)?;
    let r = create_row(&st, &ctx.org_id, &body_attrs(&raw)?).await?;
    Ok((StatusCode::CREATED, Json(json!({ "data": gcp_doc(&r.id, &r.client_email, &r.config) }))))
}

#[utoipa::path(patch, path = "/api/v2/integration/gcp/accounts/{account_id}", operation_id = "datadog_update_gcp_sts_account", params(("account_id" = String, Path)), request_body = Object, responses((status = 201, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> Result<(StatusCode, Json<Value>), DdError> {
    require(&ctx, Access::Admin)?;
    let r = update_row(&st, &ctx.org_id, id.trim_matches('"'), &body_attrs(&raw)?).await?;
    Ok((StatusCode::CREATED, Json(json!({ "data": gcp_doc(&r.id, &r.client_email, &r.config) }))))
}

#[utoipa::path(delete, path = "/api/v2/integration/gcp/accounts/{account_id}", operation_id = "datadog_delete_gcp_sts_account", params(("account_id" = String, Path)), responses((status = 204), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    delete_row(&st, &ctx.org_id, id.trim_matches('"')).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn delegate(st: &AppState) -> Result<Json<Value>, DdError> {
    let email = st.config.gcp_principal_email.clone().ok_or_else(|| DdError::new(StatusCode::NOT_FOUND, "this install has no GCP service account: set OC_GCP_PRINCIPAL_EMAIL"))?;
    Ok(Json(json!({ "data": { "id": email, "type": "gcp_sts_delegate", "attributes": { "delegate_account_email": email } } })))
}

/// The service account this install impersonates customers' service accounts with.
#[utoipa::path(get, path = "/api/v2/integration/gcp/sts_delegate", operation_id = "datadog_get_gcp_sts_delegate", responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get_delegate(State(st): State<AppState>, DdAuth(_ctx): DdAuth) -> Result<Json<Value>, DdError> {
    delegate(&st)
}

/// Datadog creates the delegate on first use; this install's is configured, so this answers it.
#[utoipa::path(post, path = "/api/v2/integration/gcp/sts_delegate", operation_id = "datadog_make_gcp_sts_delegate", responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn make_delegate(State(st): State<AppState>, DdAuth(_ctx): DdAuth) -> Result<Json<Value>, DdError> {
    delegate(&st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use serde_json::json;

    fn attrs(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn a_service_account_gets_every_attribute_the_provider_reads_back() {
        let (email, config) = gcp_input(&attrs(json!({ "client_email": "observecraft@example-project.iam.gserviceaccount.com", "host_filters": [] })), None).unwrap();
        assert_eq!(email, "observecraft@example-project.iam.gserviceaccount.com");
        assert_eq!(
            config,
            json!({
                "automute": false, "is_cspm_enabled": false, "is_security_command_center_enabled": false, "is_resource_change_collection_enabled": false,
                "is_per_project_quota_enabled": false, "resource_collection_enabled": false, "is_org_folder_resource_collection_enabled": false,
                "is_global_location_enabled": true, "account_tags": [], "host_filters": [], "cloud_run_revision_filters": [], "region_filter_configs": [],
                "metric_namespace_configs": [], "monitored_resource_configs": []
            })
        );
    }

    #[test]
    fn sent_attributes_are_kept_and_an_update_keeps_the_others() {
        let (_, config) = gcp_input(&attrs(json!({ "client_email": "sa@p.iam.gserviceaccount.com", "automute": true, "host_filters": ["env:prod"], "monitored_resource_configs": [{ "type": "gce_instance", "filters": ["env:prod"] }] })), None).unwrap();
        assert_eq!(config["automute"], json!(true));
        assert_eq!(config["monitored_resource_configs"], json!([{ "type": "gce_instance", "filters": ["env:prod"] }]));
        let (email, updated) = gcp_input(&attrs(json!({ "host_filters": ["foo:bar"] })), Some(&json!({ "client_email": "sa@p.iam.gserviceaccount.com", "config": config }))).unwrap();
        assert_eq!(email, "sa@p.iam.gserviceaccount.com");
        assert_eq!((updated["host_filters"].clone(), updated["automute"].clone()), (json!(["foo:bar"]), json!(true)));
    }

    #[test]
    fn invalid_attributes_are_refused_as_datadog_does() {
        let e = gcp_input(&attrs(json!({ "client_email": "sa@p.iam.gserviceaccount.com", "is_cspm_enabled": true, "resource_collection_enabled": false })), None).unwrap_err();
        assert_eq!((e.status, e.errors[0].as_str()), (StatusCode::BAD_REQUEST, "Resource Collection must be enabled for CSM to be enabled"));
        assert_eq!(gcp_input(&attrs(json!({})), None).unwrap_err().status, StatusCode::BAD_REQUEST);
        assert_eq!(gcp_input(&attrs(json!({ "client_email": "sa@p.iam.gserviceaccount.com", "host_filters": "env:prod" })), None).unwrap_err().status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_document_is_a_gcp_service_account() {
        let (email, config) = gcp_input(&attrs(json!({ "client_email": "sa@p.iam.gserviceaccount.com" })), None).unwrap();
        let doc = gcp_doc("e6f0237e-b9c2-4513-9940-d2473502e39e", &email, &config);
        assert_eq!((doc["type"].as_str(), doc["id"].as_str()), (Some("gcp_service_account"), Some("e6f0237e-b9c2-4513-9940-d2473502e39e")));
        assert_eq!(doc["attributes"]["client_email"], "sa@p.iam.gserviceaccount.com");
        assert_eq!(doc["attributes"]["automute"], false);
    }

    #[tokio::test]
    async fn service_accounts_belong_to_their_org_and_their_email_is_unique() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "gcp").await, crate::test_support::new_org(st, "gcp-other").await);
        let sa = attrs(json!({ "client_email": "sa@example-project.iam.gserviceaccount.com" }));
        let created = create_row(st, &org, &sa).await.unwrap();
        assert_eq!(create_row(st, &org, &sa).await.unwrap_err().status, StatusCode::CONFLICT);
        create_row(st, &other, &sa).await.unwrap();
        assert_eq!(update_row(st, &other, &created.id, &attrs(json!({ "automute": true }))).await.unwrap_err().status, StatusCode::NOT_FOUND);
        let updated = update_row(st, &org, &created.id, &attrs(json!({ "automute": true }))).await.unwrap();
        assert_eq!(updated.config["automute"], true);
        assert_eq!(delete_row(st, &org, "not-a-uuid").await.unwrap_err().status, StatusCode::BAD_REQUEST);
        delete_row(st, &org, &created.id).await.unwrap();
        assert_eq!(delete_row(st, &org, &created.id).await.unwrap_err().status, StatusCode::NOT_FOUND);
    }

    use super::super::replay::{Feature, TestServer};

    #[tokio::test]
    async fn replays_datadogs_gcp_integration_scenarios() {
        let Some(f) = Feature::load("v2/gcp_integration") else { return };
        let Some(s) = TestServer::start_with("dd-gcp-replay", |c| c.gcp_principal_email = Some("observecraft@example-project.iam.gserviceaccount.com".into())).await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= 10, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
