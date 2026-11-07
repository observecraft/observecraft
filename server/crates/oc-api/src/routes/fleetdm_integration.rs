//! Integrations → FleetDM: an org's connection to a Fleet server (`fleetdm_connections`), whose activities the server
//! polls into logs and whose hosts and policies into metrics (`crate::fleetdm`). Admins save, test and remove it and
//! ask for a sync; every member reads it and the syncs' status.
//!
//! - The token is sealed at rest, never answered, and scrubbed from errors. A saved token is only ever sent to the
//!   URL it was saved for: changing the URL needs the token again.
//! - Requests to Fleet go through the Developer Portal connectors' client and its SSRF guard
//!   (`OC_PORTAL_ALLOW_PRIVATE` for internal servers).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::dashboards::EnabledChange;
use super::portal_sources::check_site_url;
use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::fleetdm::api::{self as fleet, FleetApi, FleetError, Rest};
use crate::fleetdm::crawler::{Connection, ACTIVITIES, HOSTS, PROVIDER};
use crate::state::{AppState, OrgId};

const MAX_TOKEN_CHARS: usize = 4096;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FleetdmInput {
    /// The Fleet server's URL (`https://fleet.example.com`).
    pub base_url: String,
    /// The API token of an API-only user (the observer role is enough); leave it out to keep the saved one.
    pub token: Option<String>,
}

/// The connection as saved; the token is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetdmConnection {
    pub base_url: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FleetdmStatus {
    /// None until an admin connects Fleet.
    pub connection: Option<FleetdmConnection>,
    /// Epoch ms of the last successful poll of the activities.
    pub last_sync_at: Option<i64>,
    /// Epoch ms: activities created up to here were read.
    pub synced_until: Option<i64>,
    /// Activities sent by the last poll.
    pub activities_last_sync: i32,
    /// Epoch ms of the last successful poll of the hosts and policies.
    pub hosts_synced_at: Option<i64>,
    /// Hosts read by the last poll.
    pub hosts: i32,
    /// Why the last poll of the activities or the hosts failed (token refused, role missing, …), or its warning.
    pub error: Option<CrawlIssue>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetdmTest {
    pub ok: bool,
    pub message: String,
    /// The token's user (name or email).
    pub user: Option<String>,
    /// Its global role (observer, maintainer, admin, …), when it has one.
    pub role: Option<String>,
    /// Hosts enrolled in Fleet.
    pub hosts: Option<u64>,
}

/// The input, checked: the URL (no token).
pub fn validate(input: &FleetdmInput, allow_private: bool) -> Result<Connection, String> {
    let raw = input.base_url.trim();
    if raw.is_empty() {
        return Err("Fleet needs its URL (https://fleet.example.com)".into());
    }
    Ok(Connection { base_url: check_site_url(raw, allow_private)?, token: String::new() })
}

fn checked_token(input: &FleetdmInput) -> Result<Option<String>, ApiError> {
    let token = input.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if token.is_some_and(|t| t.len() > MAX_TOKEN_CHARS || t.chars().any(char::is_control)) {
        return Err(ApiError::BadRequest("The token is not valid".into()));
    }
    Ok(token.map(str::to_string))
}

/// The token to use: the one entered, or the saved one when the URL didn't change.
async fn token_for(st: &AppState, org: &str, input: &FleetdmInput, base_url: &str) -> Result<String, ApiError> {
    if let Some(t) = checked_token(input)? {
        return Ok(t);
    }
    match crate::fleetdm::crawler::connection(st, org).await? {
        Some((_, saved)) if saved.base_url == base_url => Ok(saved.token),
        Some(_) => Err(ApiError::BadRequest("Enter the token again: the Fleet URL changed".into())),
        None => Err(ApiError::BadRequest("Fleet needs an API token".into())),
    }
}

/// Tests a connection: the token's user, and whether it can read the hosts and the activities.
pub async fn check(api: &dyn FleetApi, conn: &Connection) -> FleetdmTest {
    let failed = |message: String, user: Option<String>, role: Option<String>| FleetdmTest { ok: false, message, user, role, hosts: None };
    let (user, role) = match api.get(&conn.base_url, &conn.token, fleet::ME).await {
        Ok(me) => {
            let u = &me["user"];
            let name = u["name"].as_str().filter(|s| !s.is_empty()).or_else(|| u["email"].as_str()).unwrap_or_default().to_string();
            (name, u["global_role"].as_str().filter(|s| !s.is_empty()).map(str::to_string))
        }
        Err(e @ FleetError::NotFound(_)) => return failed(format!("No Fleet API at this URL: {}", e.message()), None, None),
        Err(e) => return failed(format!("Fleet refused the token: {}", e.message()), None, None),
    };
    let hosts = match api.get(&conn.base_url, &conn.token, fleet::HOST_SUMMARY).await {
        Ok(s) => s["totals_hosts_count"].as_u64().unwrap_or(0),
        Err(e) => return failed(format!("{user} can't read the hosts: {}", e.message()), Some(user), role),
    };
    match api.get(&conn.base_url, &conn.token, &fleet::activities_path(0)).await {
        Ok(_) => {
            let who = role.as_deref().map_or_else(|| user.clone(), |r| format!("{user} ({r})"));
            FleetdmTest { ok: true, message: format!("Connected as {who}: {hosts} hosts"), user: Some(user), role, hosts: Some(hosts) }
        }
        Err(FleetError::Auth(_)) => failed(format!("{user} can't read the activities: give it a global role (observer is enough)"), Some(user), role),
        Err(e) => failed(format!("Reading the activities failed: {}", e.message()), Some(user), role),
    }
}

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, FleetdmConnection)>, sqlx::Error> {
    let row: Option<(String, String, i64)> = sqlx::query_as("SELECT id::text, base_url, (extract(epoch FROM updated_at) * 1000)::bigint FROM fleetdm_connections WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(|(id, base_url, updated_at)| (id, FleetdmConnection { base_url, updated_at })))
}

/// The org's connection and its syncs' status.
pub async fn status(st: &AppState, org: &str) -> Result<FleetdmStatus, sqlx::Error> {
    let mut out = FleetdmStatus { connection: None, last_sync_at: None, synced_until: None, activities_last_sync: 0, hosts_synced_at: None, hosts: 0, error: None };
    let Some((id, connection)) = saved(st, org).await? else { return Ok(out) };
    out.connection = Some(connection);
    let mut errors = Vec::new();
    for s in jobs::status(st, PROVIDER, org, &id).await? {
        match s.scope.as_str() {
            ACTIVITIES => (out.last_sync_at, out.synced_until, out.activities_last_sync) = (s.last_success_at, s.collected_until, s.metrics),
            HOSTS => (out.hosts_synced_at, out.hosts) = (s.last_success_at, s.metrics),
            _ => continue,
        }
        errors.extend(s.error);
    }
    // The latest problem of either job.
    out.error = errors.into_iter().max_by_key(|e| e.at);
    Ok(out)
}

/// Asks the poller to run the org's jobs now.
async fn poll_now(st: &AppState, org: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&st.postgres).await?.rows_affected())
}

/// The FleetDM connection and its syncs' status (no token).
#[utoipa::path(get, path = "/api/v1/integrations/fleetdm", operation_id = "fleetdm_status", responses((status = 200, body = FleetdmStatus)), tag = "integrations")]
pub async fn get_fleetdm(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<FleetdmStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the FleetDM connection (admins). The token is sealed; leave it out to keep the saved one (only while the
/// URL stays the same).
#[utoipa::path(put, path = "/api/v1/integrations/fleetdm", operation_id = "fleetdm_save", request_body = FleetdmInput,
    responses((status = 200, body = FleetdmStatus), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_fleetdm(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<FleetdmInput>) -> ApiResult<FleetdmStatus> {
    let conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    let previous = saved(&st, &ctx.org_id).await?;
    // The FleetDM dashboard is listed once a connection exists.
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO fleetdm_connections (org_id, base_url, token_enc, updated_by) VALUES ($1, $2, $3, $4::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET base_url = EXCLUDED.base_url, token_enc = EXCLUDED.token_enc, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&conn.base_url)
    .bind(st.secrets.encrypt(&token))
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    // Another server: its activities start from the first sync's lookback again.
    if previous.as_ref().is_some_and(|(_, p)| p.base_url != conn.base_url) {
        sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    }
    // The jobs now, rather than at the poller's next planning (which keeps them).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, org_id, id, s FROM fleetdm_connections, unnest($3::text[]) AS s WHERE org_id = $2 ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind([ACTIVITIES, HOSTS])
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    poll_now(&st, &ctx.org_id).await?;
    let detail = format!("{}{}", conn.base_url, if input.token.as_deref().is_some_and(|t| !t.trim().is_empty()) { ", token replaced" } else { "" });
    ctx.audit(&st, "integrations.fleetdm_saved", "fleetdm", &detail).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the FleetDM connection, its token and its polling (admins). Stored logs and metrics stay.
#[utoipa::path(delete, path = "/api/v1/integrations/fleetdm", operation_id = "fleetdm_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_fleetdm(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("DELETE FROM fleetdm_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Fleet is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integrations.fleetdm_deleted", "fleetdm", "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): the token's user and whether it can read the hosts and
/// the activities.
#[utoipa::path(post, path = "/api/v1/integrations/fleetdm/test", operation_id = "fleetdm_test", request_body = FleetdmInput,
    responses((status = 200, body = FleetdmTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_fleetdm(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<FleetdmInput>) -> ApiResult<FleetdmTest> {
    let mut conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    conn.token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    Ok(Json(check(&Rest { allow_private: st.config.portal_allow_private }, &conn).await))
}

/// Polls Fleet now instead of within the minute (admins).
#[utoipa::path(post, path = "/api/v1/integrations/fleetdm/sync", operation_id = "fleetdm_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_fleetdm(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("Fleet is not connected".into()));
    }
    poll_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleetdm::fixtures::{self, FakeFleet};
    use serde_json::json;

    fn input(v: serde_json::Value) -> FleetdmInput {
        serde_json::from_value(v).unwrap()
    }

    fn conn() -> Connection {
        Connection { base_url: "https://fleet.example.com".into(), token: "fleet-test-token".into() }
    }

    #[test]
    fn the_url_is_validated_with_the_portal_url_rules() {
        assert_eq!(validate(&input(json!({"baseUrl": " https://fleet.example.com/ "})), false).unwrap().base_url, "https://fleet.example.com");
        for (body, needle) in [
            (json!({}), "URL"),
            (json!({"baseUrl": "http://fleet.example.com"}), "https"),
            (json!({"baseUrl": "https://user:pw@fleet.example.com"}), "credentials"),
            (json!({"baseUrl": "https://10.0.0.5"}), ""),
        ] {
            let err = validate(&input(body.clone()), false).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
        assert!(validate(&input(json!({"baseUrl": "http://10.0.0.5:8080"})), true).is_ok());
    }

    #[tokio::test]
    async fn testing_reports_the_user_its_role_and_the_hosts() {
        let api = FakeFleet::default();
        api.answer(fleet::ME, json!({"user": {"name": "ObserveCraft", "email": "observecraft@example.com", "global_role": "observer", "api_only": true}}));
        api.answer(fleet::HOST_SUMMARY, fixtures::host_summary());
        api.answer(fleet::activities_path(0), fixtures::page(vec![], false));
        let t = check(&api, &conn()).await;
        assert_eq!(t, FleetdmTest { ok: true, message: "Connected as ObserveCraft (observer): 3 hosts".into(), user: Some("ObserveCraft".into()), role: Some("observer".into()), hosts: Some(3) });
    }

    #[tokio::test]
    async fn a_refused_token_is_reported_without_echoing_it() {
        let t = check(&FakeFleet { refuse: true, ..Default::default() }, &conn()).await;
        assert!(!t.ok && t.message.starts_with("Fleet refused the token"), "{t:?}");
        assert!(!serde_json::to_string(&t).unwrap().contains("fleet-test-token"));
        // A URL that isn't Fleet's API.
        let t = check(&FakeFleet::default(), &conn()).await;
        assert!(!t.ok && t.message.starts_with("No Fleet API"), "{t:?}");
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_token() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "fleetdm-a").await;
        let b = crate::test_support::admin_of_new_org(st, "fleetdm-b").await;
        let token = "fleet-SuperSecret-123";
        let saved_a = put_fleetdm(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://fleet.example.com", "token": token})))).await.unwrap().0;
        assert_eq!(saved_a.connection.as_ref().unwrap().base_url, "https://fleet.example.com");
        assert!(!serde_json::to_string(&saved_a).unwrap().contains(token));
        let sealed: String = sqlx::query_scalar("SELECT token_enc FROM fleetdm_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains(token));
        // Saving creates both jobs right away.
        let mut scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND next_run_at <= now()").bind(PROVIDER).bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        scopes.sort();
        assert_eq!(scopes, [ACTIVITIES, HOSTS]);
        // Its dashboard is listed.
        assert!(super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains("fleetdm"));

        // Org B sees nothing of A's, can't delete or sync it, and its first save needs a token.
        assert!(get_fleetdm(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_fleetdm(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_fleetdm(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        let err = put_fleetdm(State(st.clone()), Admin(b.clone()), Json(input(json!({"baseUrl": "https://fleet.example.com"})))).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(!super::super::dashboards::enabled_integrations(st, &b.org_id).await.unwrap().contains("fleetdm"));

        // A keeps its token while the URL stays; a new URL needs it again.
        let _ = put_fleetdm(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://fleet.example.com/"})))).await.unwrap();
        let (id, c) = crate::fleetdm::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!(c.token, token);
        let err = put_fleetdm(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://fleet.other.example.com"})))).await.unwrap_err();
        assert!(err.to_string().contains("token again") && !err.to_string().contains(token), "{err}");

        // The status reads both jobs.
        let job = |scope: &str| jobs::JobKey { provider: PROVIDER.into(), org_id: a.org_id.clone(), account_id: id.clone(), scope: scope.into(), collected_until: None };
        jobs::record(st, &job(ACTIVITIES), &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 7, points: 7, warning: None }, 60).await.unwrap();
        jobs::record(st, &job(HOSTS), &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 42, points: 600, warning: None }, 60).await.unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.synced_until, s.activities_last_sync, s.hosts, s.error.is_none()), (Some(1_793_700_000_000), 7, 42, true));
        assert!(s.last_sync_at.is_some() && s.hosts_synced_at.is_some());

        assert_eq!(delete_fleetdm(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_fleetdm(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
    }
}
