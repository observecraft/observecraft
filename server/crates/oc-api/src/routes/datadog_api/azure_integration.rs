//! The Azure integration, Datadog-compatible: app registrations (`/api/v1/integration/azure`, the
//! Terraform provider's `datadog_integration_azure`), on Postgres `azure_accounts`.
//!
//! - An app registration is identified by its tenant and client id. POST creates one (or replaces the
//!   one with the same identity, as Datadog adds to the configuration), PUT replaces one and can rename
//!   it (`new_tenant_name`, `new_client_id`), DELETE removes it (removing a missing one succeeds, as
//!   Datadog's recordings show), POST `/host_filters` changes its filters only.
//! - Fields get the Terraform provider's defaults; the client secret is sealed and never answered. The
//!   provider sends `SECRETLESS_AUTH` instead of a secret with secretless authentication: it keeps the
//!   stored secret (ObserveCraft has no federated identity for Azure; the crawler reports such
//!   registrations).
//! - Writes need an admin. Errors are `{"errors": ["…"]}`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};

use super::users::require;
use super::{DdAuth, DdError};
use crate::auth::Access;
use crate::state::AppState;

/// Checked, normalized fields of a create or update.
#[derive(Debug, Clone, PartialEq)]
pub struct AzureInput {
    pub tenant_name: String,
    pub client_id: String,
    /// An update's new identity (`new_tenant_name`, `new_client_id`).
    pub new_tenant_name: Option<String>,
    pub new_client_id: Option<String>,
    /// None: keep the stored one (not sent, or the provider's `SECRETLESS_AUTH` placeholder).
    pub client_secret: Option<String>,
    /// Every other field, with the Terraform provider's defaults.
    pub config: Value,
}

fn bad(msg: &str) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, msg)
}

const BOOLS: [(&str, bool); 8] = [
    ("automute", false),
    ("resource_collection_enabled", false),
    ("cspm_enabled", false),
    ("custom_metrics_enabled", false),
    ("metrics_enabled", true),
    ("metrics_enabled_default", true),
    ("usage_metrics_enabled", true),
    ("secretless_auth_enabled", false),
];
const FILTERS: [&str; 3] = ["host_filters", "container_app_filters", "app_service_plan_filters"];

fn text(body: &Map<String, Value>, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Checks a create or update body and fills the provider's defaults in.
pub fn azure_input(body: &Map<String, Value>) -> Result<AzureInput, DdError> {
    let (Some(tenant_name), Some(client_id)) = (text(body, "tenant_name"), text(body, "client_id")) else {
        return Err(bad("tenant_name and client_id are required"));
    };
    let mut config = Map::new();
    for (key, default) in BOOLS {
        let v = match body.get(key) {
            None | Some(Value::Null) => default,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(bad(&format!("{key} must be a boolean"))),
        };
        config.insert(key.into(), json!(v));
    }
    for key in FILTERS {
        let v = match body.get(key) {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.trim().to_string(),
            Some(_) => return Err(bad(&format!("{key} must be a string"))),
        };
        config.insert(key.into(), json!(v));
    }
    let mut providers = vec![];
    for c in body.get("resource_provider_configs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let namespace = c.get("namespace").and_then(Value::as_str).ok_or_else(|| bad("resource_provider_configs.namespace is required"))?;
        providers.push(json!({ "namespace": namespace, "metrics_enabled": c.get("metrics_enabled").and_then(Value::as_bool).unwrap_or(true) }));
    }
    config.insert("resource_provider_configs".into(), Value::Array(providers));
    Ok(AzureInput {
        tenant_name,
        client_id,
        new_tenant_name: text(body, "new_tenant_name"),
        new_client_id: text(body, "new_client_id"),
        client_secret: text(body, "client_secret").filter(|s| s != "SECRETLESS_AUTH"),
        config: Value::Object(config),
    })
}

/// An app registration as Datadog's API answers it (never the secret).
pub fn azure_doc(tenant_name: &str, client_id: &str, config: &Value) -> Value {
    let mut doc = config.as_object().cloned().unwrap_or_default();
    doc.insert("tenant_name".into(), json!(tenant_name));
    doc.insert("client_id".into(), json!(client_id));
    doc.insert("errors".into(), json!([]));
    Value::Object(doc)
}

// ================================================================ storage

/// A stored app registration.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct AzureRow {
    pub id: String,
    pub tenant_name: String,
    pub client_id: String,
    pub config: Value,
    pub has_secret: bool,
}

const SELECT: &str = "SELECT id::text AS id, tenant_name, client_id, config, client_secret_enc IS NOT NULL AS has_secret FROM azure_accounts";

fn db(e: sqlx::Error) -> DdError {
    tracing::error!(error = %e, "Azure integration request failed");
    DdError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

pub async fn list_rows(st: &AppState, org: &str) -> Result<Vec<AzureRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 ORDER BY created_at, id")).bind(org).fetch_all(&st.postgres).await
}

pub async fn get_row(st: &AppState, org: &str, id: &str) -> Result<Option<AzureRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await
}

/// The client secret of an app registration, unsealed.
pub async fn client_secret(st: &AppState, org: &str, id: &str) -> Result<Option<String>, sqlx::Error> {
    let sealed: Option<Option<String>> = sqlx::query_scalar("SELECT client_secret_enc FROM azure_accounts WHERE org_id = $1 AND id::text = $2").bind(org).bind(id).fetch_optional(&st.postgres).await?;
    Ok(sealed.flatten().and_then(|s| st.secrets.decrypt(&s).ok()))
}

/// Creates the app registration, or replaces the one with the same identity.
pub async fn upsert_row(st: &AppState, org: &str, body: &Map<String, Value>) -> Result<AzureRow, DdError> {
    let _changed = crate::routes::dashboards::EnabledChange(org);
    let i = azure_input(body)?;
    let sealed = i.client_secret.as_deref().map(|s| st.secrets.encrypt(s));
    sqlx::query_as(
        "INSERT INTO azure_accounts (org_id, tenant_name, client_id, client_secret_enc, config) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (org_id, tenant_name, client_id) DO UPDATE SET client_secret_enc = COALESCE(EXCLUDED.client_secret_enc, azure_accounts.client_secret_enc), \
         config = EXCLUDED.config, modified_at = now() \
         RETURNING id::text AS id, tenant_name, client_id, config, client_secret_enc IS NOT NULL AS has_secret",
    )
    .bind(org)
    .bind(&i.tenant_name)
    .bind(&i.client_id)
    .bind(sealed)
    .bind(&i.config)
    .fetch_one(&st.postgres)
    .await
    .map_err(db)
}

/// Replaces the app registration `tenant_name` + `client_id` (renamed to `new_*` when sent).
pub async fn update_row(st: &AppState, org: &str, body: &Map<String, Value>) -> Result<AzureRow, DdError> {
    let i = azure_input(body)?;
    let sealed = i.client_secret.as_deref().map(|s| st.secrets.encrypt(s));
    let r = sqlx::query_as(
        "UPDATE azure_accounts SET tenant_name = $4, client_id = $5, client_secret_enc = COALESCE($6, client_secret_enc), config = $7, modified_at = now() \
         WHERE org_id = $1 AND tenant_name = $2 AND client_id = $3 \
         RETURNING id::text AS id, tenant_name, client_id, config, client_secret_enc IS NOT NULL AS has_secret",
    )
    .bind(org)
    .bind(&i.tenant_name)
    .bind(&i.client_id)
    .bind(i.new_tenant_name.as_deref().unwrap_or(&i.tenant_name))
    .bind(i.new_client_id.as_deref().unwrap_or(&i.client_id))
    .bind(sealed)
    .bind(&i.config)
    .fetch_optional(&st.postgres)
    .await;
    match r {
        Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") => Err(DdError::new(StatusCode::CONFLICT, "an Azure app registration with this tenant and client id already exists")),
        r => r.map_err(db)?.ok_or_else(|| DdError::new(StatusCode::NOT_FOUND, "Azure app registration not found")),
    }
}

pub async fn delete_row(st: &AppState, org: &str, tenant_name: &str, client_id: &str) -> Result<u64, sqlx::Error> {
    let _changed = crate::routes::dashboards::EnabledChange(org);
    Ok(sqlx::query("DELETE FROM azure_accounts WHERE org_id = $1 AND tenant_name = $2 AND client_id = $3").bind(org).bind(tenant_name).bind(client_id).execute(&st.postgres).await?.rows_affected())
}

// ================================================================ handlers

fn json_body(raw: &Bytes) -> Result<Map<String, Value>, DdError> {
    serde_json::from_slice::<Value>(raw).ok().and_then(|v| v.as_object().cloned()).ok_or_else(|| bad("Invalid JSON body"))
}

#[utoipa::path(get, path = "/api/v1/integration/azure", operation_id = "datadog_list_azure_integration", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> Result<Json<Value>, DdError> {
    let rows = list_rows(&st, &ctx.org_id).await.map_err(db)?;
    Ok(Json(Value::Array(rows.iter().map(|r| azure_doc(&r.tenant_name, &r.client_id, &r.config)).collect())))
}

#[utoipa::path(post, path = "/api/v1/integration/azure", operation_id = "datadog_create_azure_integration", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> Result<Json<Value>, DdError> {
    require(&ctx, Access::Admin)?;
    upsert_row(&st, &ctx.org_id, &json_body(&raw)?).await?;
    Ok(Json(json!({})))
}

#[utoipa::path(put, path = "/api/v1/integration/azure", operation_id = "datadog_update_azure_integration", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> Result<Json<Value>, DdError> {
    require(&ctx, Access::Admin)?;
    update_row(&st, &ctx.org_id, &json_body(&raw)?).await?;
    Ok(Json(json!({})))
}

#[utoipa::path(delete, path = "/api/v1/integration/azure", operation_id = "datadog_delete_azure_integration", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> Result<Json<Value>, DdError> {
    require(&ctx, Access::Admin)?;
    let body = json_body(&raw)?;
    let (Some(tenant), Some(client)) = (text(&body, "tenant_name"), text(&body, "client_id")) else { return Err(bad("tenant_name and client_id are required")) };
    delete_row(&st, &ctx.org_id, &tenant, &client).await.map_err(db)?;
    Ok(Json(json!({})))
}

/// Changes the host, App Service plan and container app filters only.
#[utoipa::path(post, path = "/api/v1/integration/azure/host_filters", operation_id = "datadog_update_azure_host_filters", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn host_filters(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> Result<Json<Value>, DdError> {
    require(&ctx, Access::Admin)?;
    let body = json_body(&raw)?;
    let (Some(tenant), Some(client)) = (text(&body, "tenant_name"), text(&body, "client_id")) else { return Err(bad("tenant_name and client_id are required")) };
    let current = list_rows(&st, &ctx.org_id).await.map_err(db)?.into_iter().find(|r| r.tenant_name == tenant && r.client_id == client).ok_or_else(|| DdError::new(StatusCode::NOT_FOUND, "Azure app registration not found"))?;
    let mut merged = current.config.as_object().cloned().unwrap_or_default();
    for key in FILTERS {
        if let Some(v) = body.get(key) {
            merged.insert(key.into(), v.clone());
        }
    }
    merged.insert("tenant_name".into(), json!(tenant));
    merged.insert("client_id".into(), json!(client));
    update_row(&st, &ctx.org_id, &merged).await?;
    Ok(Json(json!({})))
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use serde_json::json;

    fn body(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn a_minimal_app_registration_gets_the_providers_defaults() {
        let i = azure_input(&body(json!({ "tenant_name": "tenant-1", "client_id": "client-1", "client_secret": "s3cret" }))).unwrap();
        assert_eq!((i.tenant_name.as_str(), i.client_id.as_str(), i.client_secret.as_deref()), ("tenant-1", "client-1", Some("s3cret")));
        assert_eq!(
            i.config,
            json!({
                "automute": false, "resource_collection_enabled": false, "cspm_enabled": false, "custom_metrics_enabled": false,
                "host_filters": "", "container_app_filters": "", "app_service_plan_filters": "",
                "metrics_enabled": true, "metrics_enabled_default": true, "usage_metrics_enabled": true, "secretless_auth_enabled": false,
                "resource_provider_configs": []
            })
        );
    }

    #[test]
    fn sent_fields_are_kept_and_the_secretless_placeholder_is_not_a_secret() {
        let i = azure_input(&body(json!({
            "tenant_name": "t", "client_id": "c", "client_secret": "SECRETLESS_AUTH", "automute": true, "host_filters": "env:prod",
            "secretless_auth_enabled": true, "new_tenant_name": "t2", "new_client_id": "c2",
            "resource_provider_configs": [{ "namespace": "Microsoft.Compute", "metrics_enabled": false }]
        })))
        .unwrap();
        assert_eq!(i.client_secret, None);
        assert_eq!((i.new_tenant_name.as_deref(), i.new_client_id.as_deref()), (Some("t2"), Some("c2")));
        assert_eq!(i.config["automute"], json!(true));
        assert_eq!(i.config["host_filters"], json!("env:prod"));
        assert_eq!(i.config["resource_provider_configs"], json!([{ "namespace": "Microsoft.Compute", "metrics_enabled": false }]));
    }

    #[test]
    fn the_identity_is_required() {
        let e = azure_input(&body(json!({ "client_id": "c" }))).unwrap_err();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert!(azure_input(&body(json!({ "tenant_name": "t", "client_id": "c", "automute": "yes" }))).is_err());
    }

    #[test]
    fn the_document_has_every_field_but_the_secret() {
        let i = azure_input(&body(json!({ "tenant_name": "t", "client_id": "c", "client_secret": "s3cret", "automute": true }))).unwrap();
        let doc = azure_doc("t", "c", &i.config);
        assert_eq!(doc["tenant_name"], "t");
        assert_eq!(doc["client_id"], "c");
        assert_eq!(doc["automute"], true);
        assert_eq!(doc["errors"], json!([]));
        assert!(doc.get("client_secret").is_none());
    }

    #[tokio::test]
    async fn app_registrations_belong_to_their_org_keep_their_secret_and_can_be_renamed() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "azure").await, crate::test_support::new_org(st, "azure-other").await);
        let created = upsert_row(st, &org, &body(json!({ "tenant_name": "t", "client_id": "c", "client_secret": "s3cret" }))).await.unwrap();
        assert!(created.has_secret);
        assert_eq!(client_secret(st, &org, &created.id).await.unwrap().as_deref(), Some("s3cret"));
        assert!(list_rows(st, &other).await.unwrap().is_empty());
        // The provider's placeholder keeps the secret; a rename keeps the row.
        let renamed = update_row(st, &org, &body(json!({ "tenant_name": "t", "client_id": "c", "client_secret": "SECRETLESS_AUTH", "new_client_id": "c2", "automute": true }))).await.unwrap();
        assert_eq!((renamed.id.as_str(), renamed.client_id.as_str(), renamed.config["automute"].as_bool()), (created.id.as_str(), "c2", Some(true)));
        assert_eq!(client_secret(st, &org, &created.id).await.unwrap().as_deref(), Some("s3cret"));
        assert_eq!(update_row(st, &other, &body(json!({ "tenant_name": "t", "client_id": "c2" }))).await.unwrap_err().status, StatusCode::NOT_FOUND);
        assert_eq!(delete_row(st, &other, "t", "c2").await.unwrap(), 0);
        assert_eq!(delete_row(st, &org, "t", "c2").await.unwrap(), 1);
    }

    use super::super::replay::{Feature, TestServer};

    #[tokio::test]
    async fn replays_datadogs_azure_integration_scenarios() {
        let Some(f) = Feature::load("v1/azure_integration") else { return };
        let Some(s) = TestServer::start("dd-azure-replay").await else { return };
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
        assert!(replayed >= 3, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
