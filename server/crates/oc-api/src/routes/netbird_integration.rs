//! Integrations → NetBird: an org's connection to a NetBird management API (`netbird_connections`), whose audit
//! events and peers the server polls into logs and metrics (`crate::netbird`). Admins save, test and remove it and
//! ask for a sync; every member reads it and the syncs' status.
//!
//! - The personal access token is sealed at rest, never answered, and scrubbed from errors. A saved token is only
//!   ever sent to the URL it was saved for: changing the URL needs the token again.
//! - Requests to NetBird go through the Developer Portal connectors' client and its SSRF guard
//!   (`OC_PORTAL_ALLOW_PRIVATE` for internal management servers).
//! - Connecting enables the NetBird dashboard (`routes::dashboards`, logo `netbird`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::dashboards::EnabledChange;
use super::portal_sources::check_site_url;
use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::netbird::api::{self as nb, NetbirdApi, NetbirdError, Rest};
use crate::netbird::crawler::{Connection, EVENTS, PEERS, PROVIDER};
use crate::state::{AppState, OrgId};

const MAX_TOKEN_CHARS: usize = 4096;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetbirdInput {
    /// The management API's URL: `https://api.netbird.io` for NetBird Cloud, the server's for self-hosted NetBird.
    pub base_url: String,
    /// A personal access token of a service user allowed to read the audit log; leave it out to keep the saved one.
    pub token: Option<String>,
}

/// The connection as saved; the token is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetbirdConnection {
    pub base_url: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NetbirdStatus {
    /// None until an admin connects NetBird.
    pub connection: Option<NetbirdConnection>,
    /// Epoch ms of the last successful poll of the audit events.
    pub last_sync_at: Option<i64>,
    /// Epoch ms: events recorded up to here were read.
    pub synced_until: Option<i64>,
    /// Events sent by the last poll.
    pub events_last_sync: i32,
    /// Epoch ms of the last successful poll of the peers.
    pub peers_synced_at: Option<i64>,
    /// Peers read by the last poll.
    pub peers: i32,
    /// Why the last poll of the events or the peers failed (token refused, role missing, …), or its warning.
    pub error: Option<CrawlIssue>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetbirdTest {
    pub ok: bool,
    pub message: String,
    /// The token's user (name or email).
    pub user: Option<String>,
    /// Its role (owner, admin, auditor, …).
    pub role: Option<String>,
    /// Audit events the token can read.
    pub events: Option<u64>,
    /// Peers of the account.
    pub peers: Option<u64>,
}

/// The input, checked: the URL (no token). A trailing `/api` is dropped: the paths add it.
pub fn validate(input: &NetbirdInput, allow_private: bool) -> Result<Connection, String> {
    let raw = input.base_url.trim().trim_end_matches('/');
    let raw = raw.strip_suffix("/api").unwrap_or(raw);
    if raw.is_empty() {
        return Err("NetBird needs its management URL (https://api.netbird.io for NetBird Cloud)".into());
    }
    Ok(Connection { base_url: check_site_url(raw, allow_private)?, token: String::new() })
}

fn checked_token(input: &NetbirdInput) -> Result<Option<String>, ApiError> {
    let token = input.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if token.is_some_and(|t| t.len() > MAX_TOKEN_CHARS || t.chars().any(|c| c.is_control() || c.is_whitespace())) {
        return Err(ApiError::BadRequest("The token is not valid".into()));
    }
    Ok(token.map(str::to_string))
}

/// The token to use: the one entered, or the saved one when the URL didn't change.
async fn token_for(st: &AppState, org: &str, input: &NetbirdInput, base_url: &str) -> Result<String, ApiError> {
    if let Some(t) = checked_token(input)? {
        return Ok(t);
    }
    match crate::netbird::crawler::connection(st, org).await? {
        Some((_, saved)) if saved.base_url == base_url => Ok(saved.token),
        Some(_) => Err(ApiError::BadRequest("Enter the token again: the NetBird URL changed".into())),
        None => Err(ApiError::BadRequest("NetBird needs a personal access token".into())),
    }
}

fn count(v: &Value) -> Option<u64> {
    v.as_array().map(|a| a.len() as u64)
}

/// Tests a connection: the token's user and role, and whether it can read the audit log and the peers.
pub async fn check(api: &dyn NetbirdApi, conn: &Connection) -> NetbirdTest {
    let failed = |message: String, user: Option<String>, role: Option<String>| NetbirdTest { ok: false, message, user, role, events: None, peers: None };
    let (user, role) = match api.get(&conn.base_url, &conn.token, nb::CURRENT_USER).await {
        Ok(me) => {
            let name = ["name", "email", "id"].iter().find_map(|k| me[*k].as_str().filter(|s| !s.is_empty())).unwrap_or("the token's user").to_string();
            (name, me["role"].as_str().filter(|s| !s.is_empty()).map(str::to_string))
        }
        Err(e @ NetbirdError::NotFound(_)) => return failed(format!("No NetBird management API at this URL: {}", e.message()), None, None),
        Err(e) => return failed(format!("NetBird refused the token: {}", e.message()), None, None),
    };
    let events = match api.get(&conn.base_url, &conn.token, nb::AUDIT_EVENTS).await {
        Ok(list) => count(&list),
        Err(NetbirdError::Auth(_)) => return failed(format!("{user} can't read the audit log: give the service user the Admin or Auditor role"), Some(user), role),
        Err(e) => return failed(format!("Reading the audit log failed: {}", e.message()), Some(user), role),
    };
    let peers = match api.get(&conn.base_url, &conn.token, nb::PEERS).await {
        Ok(list) => count(&list),
        Err(e) => return failed(format!("Reading the peers failed: {}", e.message()), Some(user), role),
    };
    let as_role = role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
    NetbirdTest {
        ok: true,
        message: format!("Connected as {user}{as_role}: {} peers, {} audit events to read", peers.unwrap_or(0), events.unwrap_or(0)),
        user: Some(user),
        role,
        events,
        peers,
    }
}

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, NetbirdConnection)>, sqlx::Error> {
    let row: Option<(String, String, i64)> = sqlx::query_as("SELECT id::text, base_url, (extract(epoch FROM updated_at) * 1000)::bigint FROM netbird_connections WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(|(id, base_url, updated_at)| (id, NetbirdConnection { base_url, updated_at })))
}

/// The org's connection and its syncs' status.
pub async fn status(st: &AppState, org: &str) -> Result<NetbirdStatus, sqlx::Error> {
    let mut out = NetbirdStatus { connection: None, last_sync_at: None, synced_until: None, events_last_sync: 0, peers_synced_at: None, peers: 0, error: None };
    let Some((id, connection)) = saved(st, org).await? else { return Ok(out) };
    out.connection = Some(connection);
    let mut errors = Vec::new();
    for s in jobs::status(st, PROVIDER, org, &id).await? {
        match s.scope.as_str() {
            EVENTS => (out.last_sync_at, out.synced_until, out.events_last_sync) = (s.last_success_at, s.collected_until, s.metrics),
            PEERS => (out.peers_synced_at, out.peers) = (s.last_success_at, s.metrics),
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

/// The NetBird connection and its syncs' status (no token).
#[utoipa::path(get, path = "/api/v1/integrations/netbird", operation_id = "netbird_status", responses((status = 200, body = NetbirdStatus)), tag = "integrations")]
pub async fn get_netbird(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<NetbirdStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the NetBird connection (admins). The token is sealed; leave it out to keep the saved one (only while the
/// URL stays the same).
#[utoipa::path(put, path = "/api/v1/integrations/netbird", operation_id = "netbird_save", request_body = NetbirdInput,
    responses((status = 200, body = NetbirdStatus), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_netbird(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<NetbirdInput>) -> ApiResult<NetbirdStatus> {
    let conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    let previous = saved(&st, &ctx.org_id).await?;
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO netbird_connections (org_id, base_url, token_enc, updated_by) VALUES ($1, $2, $3, $4::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET base_url = EXCLUDED.base_url, token_enc = EXCLUDED.token_enc, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&conn.base_url)
    .bind(st.secrets.encrypt(&token))
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    // Another server: its events start from the first sync's lookback again.
    if previous.as_ref().is_some_and(|(_, p)| p.base_url != conn.base_url) {
        sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    }
    // The jobs now, rather than at the poller's next planning (which keeps them).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, org_id, id, s FROM netbird_connections, unnest($3::text[]) AS s WHERE org_id = $2 ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind([EVENTS, PEERS])
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    poll_now(&st, &ctx.org_id).await?;
    let detail = format!("{}{}", conn.base_url, if input.token.as_deref().is_some_and(|t| !t.trim().is_empty()) { ", token replaced" } else { "" });
    ctx.audit(&st, "integrations.netbird_saved", "netbird", &detail).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the NetBird connection, its token and its polling (admins). Stored logs and metrics stay.
#[utoipa::path(delete, path = "/api/v1/integrations/netbird", operation_id = "netbird_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_netbird(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("DELETE FROM netbird_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("NetBird is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integrations.netbird_deleted", "netbird", "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): the token's user and role, and whether it can read the
/// audit log and the peers.
#[utoipa::path(post, path = "/api/v1/integrations/netbird/test", operation_id = "netbird_test", request_body = NetbirdInput,
    responses((status = 200, body = NetbirdTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_netbird(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<NetbirdInput>) -> ApiResult<NetbirdTest> {
    let mut conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    conn.token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    Ok(Json(check(&Rest { allow_private: st.config.portal_allow_private }, &conn).await))
}

/// Polls NetBird now instead of within the minute (admins).
#[utoipa::path(post, path = "/api/v1/integrations/netbird/sync", operation_id = "netbird_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_netbird(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("NetBird is not connected".into()));
    }
    poll_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netbird::fixtures::{self, FakeNetbird};
    use serde_json::json;

    fn input(v: Value) -> NetbirdInput {
        serde_json::from_value(v).unwrap()
    }

    fn conn() -> Connection {
        Connection { base_url: "https://netbird.example.com".into(), token: "nbp_test_token".into() }
    }

    #[test]
    fn the_url_is_validated_with_the_portal_url_rules() {
        assert_eq!(validate(&input(json!({"baseUrl": " https://netbird.example.com/ "})), false).unwrap().base_url, "https://netbird.example.com");
        assert_eq!(validate(&input(json!({"baseUrl": "https://api.netbird.io/api/"})), false).unwrap().base_url, "https://api.netbird.io", "the paths add /api");
        for (body, needle) in [
            (json!({}), "URL"),
            (json!({"baseUrl": "http://netbird.example.com"}), "https"),
            (json!({"baseUrl": "https://user:pw@netbird.example.com"}), "credentials"),
            (json!({"baseUrl": "https://10.0.0.5"}), ""),
        ] {
            let err = validate(&input(body.clone()), false).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
        assert!(validate(&input(json!({"baseUrl": "http://10.0.0.5:33073"})), true).is_ok());
    }

    #[tokio::test]
    async fn testing_reports_the_user_its_role_and_what_it_can_read() {
        let api = FakeNetbird::default();
        api.answer(nb::CURRENT_USER, json!({"id": "u1", "name": "observecraft-reader", "email": "", "role": "auditor", "is_service_user": true}));
        api.answer(nb::AUDIT_EVENTS, json!([fixtures::peer_added(1, "2026-11-06T11:00:00Z")]));
        api.answer(nb::PEERS, Value::Array(fixtures::peers()));
        let t = check(&api, &conn()).await;
        assert_eq!(
            t,
            NetbirdTest {
                ok: true,
                message: "Connected as observecraft-reader (auditor): 3 peers, 1 audit events to read".into(),
                user: Some("observecraft-reader".into()),
                role: Some("auditor".into()),
                events: Some(1),
                peers: Some(3),
            }
        );
    }

    #[tokio::test]
    async fn a_refused_token_or_a_missing_role_is_reported_without_echoing_the_token() {
        let t = check(&FakeNetbird { refuse: true, ..Default::default() }, &conn()).await;
        assert!(!t.ok && t.message.starts_with("NetBird refused the token"), "{t:?}");
        assert!(!serde_json::to_string(&t).unwrap().contains("nbp_test_token"));
        let t = check(&FakeNetbird::default(), &conn()).await;
        assert!(!t.ok && t.message.starts_with("No NetBird management API"), "{t:?}");
        let user = FakeNetbird { forbid_events: true, ..Default::default() };
        user.answer(nb::CURRENT_USER, json!({"name": "jordan.patel", "role": "user"}));
        let t = check(&user, &conn()).await;
        assert!(!t.ok && t.message.contains("Admin or Auditor"), "{t:?}");
        assert_eq!(t.role.as_deref(), Some("user"));
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_token() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "netbird-a").await;
        let b = crate::test_support::admin_of_new_org(st, "netbird-b").await;
        let token = "nbp_SuperSecret123";
        let saved_a = put_netbird(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://netbird.example.com", "token": token})))).await.unwrap().0;
        assert_eq!(saved_a.connection.as_ref().unwrap().base_url, "https://netbird.example.com");
        assert!(!serde_json::to_string(&saved_a).unwrap().contains(token));
        let sealed: String = sqlx::query_scalar("SELECT token_enc FROM netbird_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains(token));
        // Saving creates both jobs right away.
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND next_run_at <= now() ORDER BY scope").bind(PROVIDER).bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(scopes, [EVENTS, PEERS]);
        // The NetBird dashboard is enabled for A only.
        assert!(super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains("netbird"));
        assert!(!super::super::dashboards::enabled_integrations(st, &b.org_id).await.unwrap().contains("netbird"));

        // Org B sees nothing of A's, can't delete or sync it, and its first save needs a token.
        assert!(get_netbird(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_netbird(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_netbird(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        let err = put_netbird(State(st.clone()), Admin(b.clone()), Json(input(json!({"baseUrl": "https://netbird.example.com"})))).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));

        // A keeps its token while the URL stays; a new URL needs it again.
        let _ = put_netbird(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://netbird.example.com/"})))).await.unwrap();
        let (id, c) = crate::netbird::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!(c.token, token);
        let err = put_netbird(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://vpn.other.example.com"})))).await.unwrap_err();
        assert!(err.to_string().contains("token again") && !err.to_string().contains(token), "{err}");

        // The status reads both jobs.
        let job = |scope: &str| jobs::JobKey { provider: PROVIDER.into(), org_id: a.org_id.clone(), account_id: id.clone(), scope: scope.into(), collected_until: None };
        jobs::record(st, &job(EVENTS), &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 7, points: 7, warning: None }, 60).await.unwrap();
        jobs::record(st, &job(PEERS), &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 3, points: 18, warning: None }, 60).await.unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.synced_until, s.events_last_sync, s.peers, s.error.is_none()), (Some(1_793_700_000_000), 7, 3, true));
        assert!(s.last_sync_at.is_some() && s.peers_synced_at.is_some());

        assert_eq!(delete_netbird(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_netbird(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
        assert!(!super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains("netbird"));
    }
}
