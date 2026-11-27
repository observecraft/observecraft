//! Integrations → Vanta: an org's private Vanta app (`vanta_connections`), to which the server sends its compliance
//! evidence every hour (`crate::vanta`). Admins save, test and remove it and ask for a sync; every member reads it
//! and each resource type's last sync.
//!
//! - The client secret is sealed at rest, never answered, and scrubbed from errors. A saved secret is kept only while
//!   the client id stays the same: another app needs its secret.
//! - Only Vanta's hosts are called: the region picks `https://api.vanta.com` or `https://api.vanta-gov.com`.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};
use crate::vanta::api::{Rest, VantaApi, API_URLS};
use crate::vanta::sync::{base_type, Connection, PROVIDER, SCOPE, TYPES};

const MAX_FIELD_CHARS: usize = 512;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct VantaInput {
    /// `us` (api.vanta.com) or `gov` (api.vanta-gov.com).
    pub region: String,
    /// The private app's client id (Vanta → Settings → Developer Console).
    pub client_id: String,
    /// Its client secret; leave it out to keep the saved one (only while the client id stays the same).
    pub client_secret: Option<String>,
    /// The Developer Console's `resourceId` of each resource type to send (`user_account`, …); a type left out or
    /// empty isn't sent.
    pub resource_ids: HashMap<String, String>,
}

/// The connection as saved; the secret is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VantaConnection {
    pub region: String,
    pub client_id: String,
    pub resource_ids: HashMap<String, String>,
    /// Epoch ms.
    pub updated_at: i64,
}

/// A resource type's last sync.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VantaTypeStatus {
    pub resource_type: String,
    /// The Developer Console base type its resource is created with.
    pub base_type: String,
    /// For custom resources, the schema (JSON Type Definition) to create the resource with.
    pub schema: Option<String>,
    /// It has a `resourceId`, so it is sent.
    pub configured: bool,
    /// Epoch ms.
    pub synced_at: Option<i64>,
    pub sent: i32,
    pub accepted: i32,
    pub rejected: i32,
    pub error: Option<String>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VantaStatus {
    /// None until an admin connects Vanta.
    pub connection: Option<VantaConnection>,
    /// Epoch ms of the last sync that sent something.
    pub last_sync_at: Option<i64>,
    /// Why the last sync failed (credentials refused, throttled, …), or its warning.
    pub error: Option<CrawlIssue>,
    /// Every resource type ObserveCraft sends, configured or not.
    pub types: Vec<VantaTypeStatus>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VantaTest {
    pub ok: bool,
    pub message: String,
}

/// The API host of a region.
pub fn api_url(region: &str) -> Option<&'static str> {
    match region.trim() {
        "us" => Some(API_URLS[0]),
        "gov" => Some(API_URLS[1]),
        _ => None,
    }
}

fn region_of(api_url: &str) -> &'static str {
    if api_url == API_URLS[1] {
        "gov"
    } else {
        "us"
    }
}

/// The input, checked: the region's host, the client id and the resource ids (not the secret).
pub fn validate(input: &VantaInput) -> Result<(String, String, HashMap<String, String>), String> {
    let bad = |v: &str| v.len() > MAX_FIELD_CHARS || v.chars().any(char::is_control);
    let url = api_url(&input.region).ok_or("The region must be us (api.vanta.com) or gov (api.vanta-gov.com)")?;
    let client_id = input.client_id.trim();
    if client_id.is_empty() {
        return Err("Vanta needs the app's client id".into());
    }
    if bad(client_id) {
        return Err("The client id is not valid".into());
    }
    let mut ids = HashMap::new();
    for (t, id) in &input.resource_ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if !TYPES.contains(&t.as_str()) {
            return Err(format!("ObserveCraft doesn't send {t} to Vanta (it sends {})", TYPES.join(", ")));
        }
        if bad(id) {
            return Err(format!("The resourceId of {t} is not valid"));
        }
        ids.insert(t.clone(), id.to_string());
    }
    if ids.contains_key("package_vulnerability") && !ids.contains_key("vulnerable_component") {
        return Err("Vulnerabilities name their vulnerable components: send the vulnerable components too".into());
    }
    Ok((url.to_string(), client_id.to_string(), ids))
}

fn checked_secret(input: &VantaInput) -> Result<Option<String>, ApiError> {
    let secret = input.client_secret.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if secret.is_some_and(|s| s.len() > MAX_FIELD_CHARS || s.chars().any(char::is_control)) {
        return Err(ApiError::BadRequest("The client secret is not valid".into()));
    }
    Ok(secret.map(str::to_string))
}

/// The secret to use: the one entered, or the saved one when the client id didn't change.
async fn secret_for(st: &AppState, org: &str, input: &VantaInput, client_id: &str) -> Result<String, ApiError> {
    if let Some(s) = checked_secret(input)? {
        return Ok(s);
    }
    match crate::vanta::sync::connection(st, org).await? {
        Some(saved) if saved.client_id == client_id => Ok(saved.client_secret),
        Some(_) => Err(ApiError::BadRequest("Enter the client secret again: the client id changed".into())),
        None => Err(ApiError::BadRequest("Vanta needs the app's client secret".into())),
    }
}

/// Tests the credentials: whether Vanta gives the app a token. (It revokes the app's previous token, which the next
/// sync replaces.)
pub async fn check(api: &dyn VantaApi, conn: &Connection) -> VantaTest {
    match api.token(&conn.api_url, &conn.client_id, &conn.client_secret).await {
        Ok(_) => VantaTest { ok: true, message: format!("Vanta accepted the app's credentials ({})", conn.api_url) },
        Err(e) => VantaTest { ok: false, message: format!("Vanta refused the credentials: {}", e.message()) },
    }
}

/// `vanta_connections`' id, api_url, client_id, resource_ids and updated_at (epoch ms).
type SavedRow = (String, String, String, sqlx::types::Json<HashMap<String, String>>, i64);
/// `vanta_sync_results`' resource_type, synced_at (epoch ms), sent, accepted, rejected and error.
type ResultRow = (String, i64, i32, i32, i32, Option<String>);

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, VantaConnection)>, sqlx::Error> {
    let row: Option<SavedRow> =
        sqlx::query_as("SELECT id::text, api_url, client_id, resource_ids, (extract(epoch FROM updated_at) * 1000)::bigint FROM vanta_connections WHERE org_id = $1")
            .bind(org)
            .fetch_optional(&st.postgres)
            .await?;
    Ok(row.map(|(id, api_url, client_id, ids, updated_at)| (id, VantaConnection { region: region_of(&api_url).into(), client_id, resource_ids: ids.0, updated_at })))
}

/// A type's status: what creating its resource needs, and its last sync.
fn type_status(kind: &str, configured: bool, last: Option<&ResultRow>) -> VantaTypeStatus {
    let schema = crate::vanta::custom::schema(kind).map(|s| serde_json::to_string_pretty(&s).unwrap_or_default());
    let mut out = VantaTypeStatus { resource_type: kind.to_string(), base_type: base_type(kind).to_string(), schema, configured, synced_at: None, sent: 0, accepted: 0, rejected: 0, error: None };
    if let Some((_, at, sent, accepted, rejected, error)) = last {
        (out.synced_at, out.sent, out.accepted, out.rejected, out.error) = (Some(*at), *sent, *accepted, *rejected, error.clone());
    }
    out
}

/// The org's connection and each type's last sync.
pub async fn status(st: &AppState, org: &str) -> Result<VantaStatus, sqlx::Error> {
    let mut out = VantaStatus { connection: None, last_sync_at: None, error: None, types: Vec::new() };
    let Some((id, connection)) = saved(st, org).await? else {
        out.types = TYPES.iter().map(|t| type_status(t, false, None)).collect();
        return Ok(out);
    };
    let rows: Vec<ResultRow> =
        sqlx::query_as("SELECT resource_type, (extract(epoch FROM synced_at) * 1000)::bigint, sent, accepted, rejected, error FROM vanta_sync_results WHERE connection_id = $1::uuid").bind(&id).fetch_all(&st.postgres).await?;
    out.types = TYPES
        .iter()
        .map(|t| {
            let configured = connection.resource_ids.get(*t).is_some_and(|v| !v.trim().is_empty());
            type_status(t, configured, rows.iter().find(|r| r.0 == *t))
        })
        .collect();
    out.connection = Some(connection);
    if let Some(s) = jobs::status(st, PROVIDER, org, &id).await?.into_iter().find(|s| s.scope == SCOPE) {
        (out.last_sync_at, out.error) = (s.last_success_at, s.error);
    }
    Ok(out)
}

/// Asks the sync to run the org's job now.
async fn sync_now(st: &AppState, org: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&st.postgres).await?.rows_affected())
}

/// The Vanta connection and each resource type's last sync (no secret).
#[utoipa::path(get, path = "/api/v1/integrations/vanta", operation_id = "vanta_status", responses((status = 200, body = VantaStatus)), tag = "integrations")]
pub async fn get_vanta(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<VantaStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the Vanta connection (admins). The secret is sealed; leave it out to keep the saved one (only while the client
/// id stays the same).
#[utoipa::path(put, path = "/api/v1/integrations/vanta", operation_id = "vanta_save", request_body = VantaInput,
    responses((status = 200, body = VantaStatus), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_vanta(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<VantaInput>) -> ApiResult<VantaStatus> {
    let (api_url, client_id, resource_ids) = validate(&input).map_err(ApiError::BadRequest)?;
    let secret = secret_for(&st, &ctx.org_id, &input, &client_id).await?;
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO vanta_connections (org_id, api_url, client_id, client_secret_enc, resource_ids, updated_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET api_url = EXCLUDED.api_url, client_id = EXCLUDED.client_id, client_secret_enc = EXCLUDED.client_secret_enc, \
         resource_ids = EXCLUDED.resource_ids, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&api_url)
    .bind(&client_id)
    .bind(st.secrets.encrypt(&secret))
    .bind(sqlx::types::Json(&resource_ids))
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    // A type no longer sent has no last sync to show.
    sqlx::query("DELETE FROM vanta_sync_results r USING vanta_connections c WHERE r.connection_id = c.id AND c.org_id = $1 AND NOT (c.resource_ids ? r.resource_type)").bind(&ctx.org_id).execute(&mut *tx).await?;
    // The job now, rather than at the sync's next planning (which keeps it).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, org_id, id, $3 FROM vanta_connections WHERE org_id = $2 ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind(SCOPE)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    sync_now(&st, &ctx.org_id).await?;
    let mut types: Vec<&str> = resource_ids.keys().map(String::as_str).collect();
    types.sort();
    let detail = format!("{api_url}, {}{}", types.join(", "), if input.client_secret.as_deref().is_some_and(|s| !s.trim().is_empty()) { ", secret replaced" } else { "" });
    ctx.audit(&st, "integrations.vanta_saved", "vanta", &detail).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the Vanta connection, its secret and its sync (admins). What was sent stays in Vanta.
#[utoipa::path(delete, path = "/api/v1/integrations/vanta", operation_id = "vanta_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_vanta(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("DELETE FROM vanta_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Vanta is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integrations.vanta_deleted", "vanta", "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests the app's credentials before or after saving them (admins).
#[utoipa::path(post, path = "/api/v1/integrations/vanta/test", operation_id = "vanta_test", request_body = VantaInput,
    responses((status = 200, body = VantaTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_vanta(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<VantaInput>) -> ApiResult<VantaTest> {
    let (api_url, client_id, resource_ids) = validate(&input).map_err(ApiError::BadRequest)?;
    let client_secret = secret_for(&st, &ctx.org_id, &input, &client_id).await?;
    let conn = Connection { id: String::new(), api_url, client_id, client_secret, resource_ids };
    Ok(Json(check(&Rest, &conn).await))
}

/// Syncs now instead of within the hour (admins).
#[utoipa::path(post, path = "/api/v1/integrations/vanta/sync", operation_id = "vanta_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_vanta(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("Vanta is not connected".into()));
    }
    sync_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vanta::sync::fake::FakeVanta;
    use serde_json::json;

    fn input(v: serde_json::Value) -> VantaInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn only_vantas_hosts_and_known_types_are_accepted() {
        assert_eq!(api_url("us"), Some("https://api.vanta.com"));
        assert_eq!(api_url("gov"), Some("https://api.vanta-gov.com"));
        assert_eq!(api_url("https://evil.example.com"), None);
        let (url, id, ids) = validate(&input(json!({"region": "us", "clientId": " vci_example ", "resourceIds": {"user_account": " res_users ", "custom_resource": ""}}))).unwrap();
        assert_eq!((url.as_str(), id.as_str()), ("https://api.vanta.com", "vci_example"));
        assert_eq!(ids, HashMap::from([("user_account".to_string(), "res_users".to_string())]), "trimmed, empty ones dropped");
        for (body, needle) in [
            (json!({"region": "eu", "clientId": "vci"}), "region"),
            (json!({"region": "us", "clientId": ""}), "client id"),
            (json!({"region": "us", "clientId": "vci", "resourceIds": {"computers": "res_x"}}), "computers"),
            (json!({"region": "us", "clientId": "vci", "resourceIds": {"user_account": "res\nx"}}), "resourceId"),
            (json!({"region": "us", "clientId": "vci", "resourceIds": {"package_vulnerability": "res_v"}}), "vulnerable components"),
        ] {
            let err = validate(&input(body.clone())).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
    }

    #[tokio::test]
    async fn testing_reports_whether_vanta_gives_a_token_without_echoing_the_secret() {
        let conn = Connection { id: String::new(), api_url: "https://api.vanta.com".into(), client_id: "vci".into(), client_secret: "vcs_secret".into(), resource_ids: HashMap::new() };
        assert!(check(&FakeVanta::default(), &conn).await.ok);
        let t = check(&FakeVanta { refuse_token: true, ..Default::default() }, &conn).await;
        assert!(!t.ok && t.message.starts_with("Vanta refused the credentials"), "{t:?}");
        assert!(!t.message.contains("vcs_secret"));
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_secret() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "vanta-a").await;
        let b = crate::test_support::admin_of_new_org(st, "vanta-b").await;
        let secret = "vcs_SuperSecret-123";
        let body = json!({"region": "us", "clientId": "vci_example", "clientSecret": secret, "resourceIds": {"user_account": "res_users"}});
        let saved_a = put_vanta(State(st.clone()), Admin(a.clone()), Json(input(body))).await.unwrap().0;
        let conn = saved_a.connection.clone().unwrap();
        assert_eq!((conn.region.as_str(), conn.client_id.as_str()), ("us", "vci_example"));
        assert_eq!(saved_a.types.len(), TYPES.len(), "every type, configured or not");
        assert_eq!((saved_a.types[0].resource_type.as_str(), saved_a.types[0].configured, saved_a.types[0].base_type.as_str()), ("user_account", true, "UserAccount"));
        let monitor = saved_a.types.iter().find(|t| t.resource_type == "monitor").unwrap();
        assert_eq!((monitor.configured, monitor.base_type.as_str()), (false, "CustomResource"));
        assert!(monitor.schema.as_deref().unwrap().contains("\"notifies\""), "custom resources come with the schema to create them with");
        assert!(saved_a.types[0].schema.is_none());
        assert!(!serde_json::to_string(&saved_a).unwrap().contains(secret));
        let sealed: String = sqlx::query_scalar("SELECT client_secret_enc FROM vanta_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains(secret));
        // Saving creates the sync job right away.
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND next_run_at <= now()").bind(PROVIDER).bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(scopes, [SCOPE]);

        // Org B sees nothing of A's, can't delete or sync it, and its first save needs a secret.
        assert!(get_vanta(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_vanta(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_vanta(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(put_vanta(State(st.clone()), Admin(b.clone()), Json(input(json!({"region": "us", "clientId": "vci_example"})))).await, Err(ApiError::BadRequest(_))));

        // A keeps its secret while the client id stays; another app needs its own.
        let _ = put_vanta(State(st.clone()), Admin(a.clone()), Json(input(json!({"region": "gov", "clientId": "vci_example", "resourceIds": {"user_account": "res_users"}})))).await.unwrap();
        let c = crate::vanta::sync::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!((c.client_secret.as_str(), c.api_url.as_str()), (secret, "https://api.vanta-gov.com"));
        let err = put_vanta(State(st.clone()), Admin(a.clone()), Json(input(json!({"region": "us", "clientId": "vci_other"})))).await.unwrap_err();
        assert!(err.to_string().contains("secret again") && !err.to_string().contains(secret), "{err}");

        // The status shows each type's last sync; a type no longer configured loses it.
        crate::vanta::sync::save_results(st, &c.id, &[crate::vanta::sync::TypeResult { resource_type: "user_account".into(), sent: 4, accepted: 3, rejected: 1, error: Some("1 of 4 rejected".into()) }]).await.unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.types[0].sent, s.types[0].accepted, s.types[0].rejected, s.types[0].synced_at.is_some()), (4, 3, 1, true));
        let _ = put_vanta(State(st.clone()), Admin(a.clone()), Json(input(json!({"region": "gov", "clientId": "vci_example"})))).await.unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.types[0].configured, s.types[0].synced_at), (false, None));

        assert_eq!(delete_vanta(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_vanta(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
    }
}
