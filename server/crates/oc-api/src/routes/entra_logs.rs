//! Integrations → Microsoft Azure → an app registration's Entra ID logs (the Microsoft Entra ID tile): whether the
//! server reads the tenant's sign-ins, directory audits and risk detections from Microsoft Graph (`crate::entra`,
//! Postgres `entra_log_settings`), each stream's polling status, and a test of what the app registration may read.
//! Admins turn them on and off and test; every member reads them.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::dashboards::EnabledChange;
use super::integration_installs::install_pipelines;
use crate::auth::Admin;
use crate::azure::api::{Credentials, Rest};
use crate::cloud::jobs::{self, ScopeStatus};
use crate::entra::api::{list_path, GraphApi, Stream, STREAMS};
use crate::entra::crawler::{self, Settings, PROVIDER};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// The catalog tile the Entra ID logs install (its log pipeline) and count as installed.
pub const INTEGRATION: &str = "azure-active-directory";

/// An app registration's Entra ID logs.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EntraLogs {
    pub enabled: bool,
    pub sign_ins: bool,
    pub audits: bool,
    pub risk: bool,
    /// One per stream on: scope `signins`, `audits` or `risk`.
    pub streams: Vec<ScopeStatus>,
}

/// What an admin turns on.
#[derive(Deserialize, ToSchema, Debug, Clone, Copy)]
#[serde(rename_all = "camelCase", default)]
pub struct EntraLogsInput {
    pub enabled: bool,
    pub sign_ins: bool,
    pub audits: bool,
    pub risk: bool,
}

impl Default for EntraLogsInput {
    fn default() -> Self {
        EntraLogsInput { enabled: true, sign_ins: true, audits: true, risk: false }
    }
}

/// One stream's test.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntraStreamTest {
    /// `signins`, `audits` or `risk`.
    pub stream: String,
    pub ok: bool,
    pub message: String,
    /// The Microsoft Graph application permission it needs.
    pub permission: String,
    /// The license it needs, when one does.
    pub license: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntraLogsTest {
    /// Every stream readable.
    pub ok: bool,
    pub streams: Vec<EntraStreamTest>,
}

/// What the app registration may read: each stream's events of the last hour (a page).
pub async fn check(api: &dyn GraphApi, creds: &Credentials, now_ms: i64) -> EntraLogsTest {
    let mut streams = Vec::new();
    for stream in STREAMS {
        let (ok, message) = match api.get(creds, &list_path(stream, now_ms - 3_600_000, now_ms)).await {
            Ok(page) => {
                let n = page["value"].as_array().map_or(0, Vec::len);
                let more = if page["@odata.nextLink"].is_string() { " or more" } else { "" };
                (true, format!("Can read the {}: {n}{more} in the last hour", crawler::label(stream)))
            }
            Err(e) => (false, crawler::issue(stream, &e).message),
        };
        streams.push(EntraStreamTest { stream: stream.scope().into(), ok, message, permission: stream.permission().into(), license: stream.license().map(str::to_string) });
    }
    EntraLogsTest { ok: streams.iter().all(|s| s.ok), streams }
}

/// The app registration of the org (404 otherwise).
async fn account(st: &AppState, org: &str, id: &str) -> Result<(), ApiError> {
    match super::datadog_api::azure_integration::get_row(st, org, id).await? {
        Some(_) => Ok(()),
        None => Err(ApiError::NotFound("Azure app registration not found".into())),
    }
}

async fn view(st: &AppState, org: &str, id: &str) -> Result<EntraLogs, ApiError> {
    let Some(on) = crawler::settings(st, org, id).await? else { return Ok(EntraLogs { enabled: false, sign_ins: true, audits: true, risk: false, streams: vec![] }) };
    let streams = jobs::status(st, PROVIDER, org, id).await?;
    Ok(EntraLogs { enabled: true, sign_ins: on.sign_ins, audits: on.audits, risk: on.risk, streams })
}

/// The app registration's Entra ID logs and their polling status.
#[utoipa::path(get, path = "/api/v1/integrations/azure/accounts/{id}/entra-logs", operation_id = "azure_entra_logs", params(("id" = String, Path)),
    responses((status = 200, body = EntraLogs), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn get_entra_logs(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<EntraLogs> {
    account(&st, &org, &id).await?;
    Ok(Json(view(&st, &org, &id).await?))
}

/// Turns the app registration's Entra ID logs on or off, by stream (admins). On, the Microsoft Entra ID tile counts
/// as installed and its log pipeline is installed; off, the polling stops (stored logs stay).
#[utoipa::path(put, path = "/api/v1/integrations/azure/accounts/{id}/entra-logs", operation_id = "azure_entra_logs_save", params(("id" = String, Path)), request_body = EntraLogsInput,
    responses((status = 200, body = EntraLogs), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_entra_logs(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<EntraLogsInput>) -> ApiResult<EntraLogs> {
    account(&st, &ctx.org_id, &id).await?;
    let on = Settings { sign_ins: input.sign_ins, audits: input.audits, risk: input.risk };
    let enabled = input.enabled && !on.streams().is_empty();
    // The Entra ID dashboard is listed while an app registration has them on.
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    if enabled {
        sqlx::query(
            "INSERT INTO entra_log_settings (org_id, account_id, sign_ins, audits, risk, updated_by) VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid) \
             ON CONFLICT (org_id, account_id) DO UPDATE SET sign_ins = EXCLUDED.sign_ins, audits = EXCLUDED.audits, risk = EXCLUDED.risk, updated_by = EXCLUDED.updated_by, updated_at = now()",
        )
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(on.sign_ins)
        .bind(on.audits)
        .bind(on.risk)
        .bind(&ctx.user_id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query("DELETE FROM entra_log_settings WHERE org_id = $1 AND account_id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&mut *tx).await?;
    }
    // The jobs now, rather than at the poller's next planning: the streams on, run at once; the others' stop.
    let scopes: Vec<&str> = if enabled { on.streams().into_iter().map(Stream::scope).collect() } else { vec![] };
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid AND NOT (scope = ANY($4))").bind(PROVIDER).bind(&ctx.org_id).bind(&id).bind(&scopes).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, $2, $3::uuid, s FROM unnest($4::text[]) AS s ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&scopes)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if enabled {
        // The integration's library log pipeline (integrations-core's), as installing its tile would.
        install_pipelines(&st, &ctx.org_id, INTEGRATION).await?;
    }
    let detail = format!("{id}: {}", if enabled { scopes.join(", ") } else { "off".into() });
    ctx.audit(&st, "integrations.entra_logs_saved", "azure", &detail).await;
    Ok(Json(view(&st, &ctx.org_id, &id).await?))
}

/// Tests what the app registration may read from Microsoft Graph, stream by stream (admins): the permissions and
/// licenses each needs are named when Graph refuses.
#[utoipa::path(post, path = "/api/v1/integrations/azure/accounts/{id}/entra-logs/test", operation_id = "azure_entra_logs_test", params(("id" = String, Path)),
    responses((status = 200, body = EntraLogsTest), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_entra_logs(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> ApiResult<EntraLogsTest> {
    account(&st, &ctx.org_id, &id).await?;
    let Some(creds) = crawler::credentials(&st, &ctx.org_id, &id).await? else { return Err(ApiError::BadRequest("This app registration has no client secret: add one under Details".into())) };
    let api = Rest::new(reqwest::Client::new(), Default::default());
    Ok(Json(check(&api, &creds, chrono::Utc::now().timestamp_millis()).await))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::api::AzureError;
    use crate::entra::fixtures::{self, FakeGraph};
    use serde_json::json;

    fn creds() -> Credentials {
        Credentials { tenant_id: fixtures::TENANT.into(), client_id: "c".into(), client_secret: "s".into() }
    }

    #[tokio::test]
    async fn the_test_reads_each_stream_and_names_what_is_missing() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-06T12:00:00Z").unwrap().timestamp_millis();
        let api = FakeGraph::default();
        api.set("auditLogs/signIns", vec![fixtures::sign_in("s-1", "2026-11-06T11:30:00Z", "jordan.patel@example.com", 0)]);
        let t = check(&api, &creds(), now).await;
        assert!(t.ok, "{t:?}");
        assert_eq!(t.streams.iter().map(|s| s.stream.as_str()).collect::<Vec<_>>(), ["signins", "audits", "risk"]);
        assert_eq!(t.streams[0].message, "Can read the sign-in logs: 1 in the last hour");
        assert_eq!((t.streams[0].license.as_deref(), t.streams[2].permission.as_str()), (Some("Microsoft Entra ID P1 or P2"), "IdentityRiskEvent.Read.All"));
        let denied = FakeGraph { fail: Some(AzureError::Forbidden { action: None, message: "Authorization_RequestDenied: Insufficient privileges to complete the operation.".into() }), ..Default::default() };
        let t = check(&denied, &creds(), now).await;
        assert!(!t.ok && t.streams.iter().all(|s| !s.ok));
        assert!(t.streams[1].message.contains("AuditLog.Read.All"), "{}", t.streams[1].message);
    }

    #[tokio::test]
    async fn turning_entra_logs_on_creates_the_jobs_installs_the_pipeline_and_belongs_to_the_org() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "entra-a").await;
        let b = crate::test_support::admin_of_new_org(st, "entra-b").await;
        let body = json!({ "tenant_name": fixtures::TENANT, "client_id": "c", "client_secret": "s" }).as_object().cloned().unwrap();
        let Json(acct) = super::super::azure_integration::create_account(State(st.clone()), Admin(a.clone()), Json(body)).await.unwrap();
        let off = get_entra_logs(State(st.clone()), OrgId(a.org_id.clone()), Path(acct.id.clone())).await.unwrap().0;
        assert!(!off.enabled && off.streams.is_empty());
        let on = put_entra_logs(State(st.clone()), Admin(a.clone()), Path(acct.id.clone()), Json(EntraLogsInput { risk: true, ..Default::default() })).await.unwrap().0;
        assert!(on.enabled && on.risk);
        assert_eq!(on.streams.iter().map(|s| s.scope.as_str()).collect::<Vec<_>>(), ["audits", "risk", "signins"]);
        let installed = super::super::integration_installs::installed(st, &a.org_id).await.unwrap();
        assert!(installed.contains(&INTEGRATION.to_string()) && installed.contains(&"azure".to_string()), "{installed:?}");
        let pipelines: Vec<String> = sqlx::query_scalar("SELECT source FROM log_pipelines WHERE org_id = $1").bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(pipelines, ["integration:azure_active_directory"]);
        assert!(super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains(INTEGRATION));
        // The poller wants the same jobs.
        let desired = crawler::EntraCrawler { api: std::sync::Arc::new(FakeGraph::default()), sink: std::sync::Arc::new(crate::cloud::testing::FakeSink::default()) };
        let wanted: Vec<_> = jobs::Crawler::desired(&desired, st).await.unwrap().into_iter().filter(|(o, _, _)| *o == a.org_id).map(|(_, _, s)| s).collect();
        assert_eq!(wanted, ["signins", "audits", "risk"]);

        // Org B can't read or change A's.
        assert!(matches!(get_entra_logs(State(st.clone()), OrgId(b.org_id.clone()), Path(acct.id.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(put_entra_logs(State(st.clone()), Admin(b.clone()), Path(acct.id.clone()), Json(EntraLogsInput::default())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(test_entra_logs(State(st.clone()), Admin(b.clone()), Path(acct.id.clone())).await, Err(ApiError::NotFound(_))));

        // Fewer streams drop their jobs; off drops them all.
        let audits = put_entra_logs(State(st.clone()), Admin(a.clone()), Path(acct.id.clone()), Json(EntraLogsInput { sign_ins: false, ..Default::default() })).await.unwrap().0;
        assert_eq!(audits.streams.iter().map(|s| s.scope.as_str()).collect::<Vec<_>>(), ["audits"]);
        let off = put_entra_logs(State(st.clone()), Admin(a.clone()), Path(acct.id.clone()), Json(EntraLogsInput { enabled: false, ..Default::default() })).await.unwrap().0;
        assert!(!off.enabled && off.streams.is_empty());
        assert!(!super::super::integration_installs::installed(st, &a.org_id).await.unwrap().contains(&INTEGRATION.to_string()));
        // Removing the app registration removes its settings.
        let _ = put_entra_logs(State(st.clone()), Admin(a.clone()), Path(acct.id.clone()), Json(EntraLogsInput::default())).await.unwrap();
        super::super::azure_integration::delete_account(State(st.clone()), Admin(a.clone()), Path(acct.id.clone())).await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM entra_log_settings WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
    }
}
