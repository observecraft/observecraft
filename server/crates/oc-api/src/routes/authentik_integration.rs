//! Integrations → authentik: an org's connection to an authentik instance (`authentik_connections`), whose
//! events the server polls into logs (`crate::authentik`). Admins save, test and remove it and ask for a sync;
//! every member reads it and the sync's status.
//!
//! - The token is sealed at rest, never answered, and scrubbed from errors. A saved token is only ever sent to
//!   the URL it was saved for: changing the URL needs the token again.
//! - Requests to authentik go through the Developer Portal connectors' client and its SSRF guard
//!   (`OC_PORTAL_ALLOW_PRIVATE` for internal instances).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::portal_sources::check_site_url;
use crate::auth::Admin;
use crate::authentik::api::{self as ak, AuthentikApi, AuthentikError, Rest};
use crate::authentik::crawler::{Connection, PROVIDER, SCOPE};
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const MAX_TOKEN_CHARS: usize = 4096;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthentikInput {
    /// The instance's URL (`https://auth.example.com`).
    pub base_url: String,
    /// An API token of a user allowed to view events; leave it out to keep the saved one.
    pub token: Option<String>,
}

/// The connection as saved; the token is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikConnection {
    pub base_url: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikStatus {
    /// None until an admin connects authentik.
    pub connection: Option<AuthentikConnection>,
    /// Epoch ms of the last successful poll.
    pub last_sync_at: Option<i64>,
    /// Epoch ms: events created up to here were read.
    pub synced_until: Option<i64>,
    /// Events sent by the last poll.
    pub events_last_sync: i32,
    /// Why the last poll failed (token refused, permission missing, …), or its warning.
    pub error: Option<CrawlIssue>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikTest {
    pub ok: bool,
    pub message: String,
    /// The token's user.
    pub user: Option<String>,
    /// Events the token can read.
    pub events: Option<u64>,
}

/// The input, checked: the URL (no token).
pub fn validate(input: &AuthentikInput, allow_private: bool) -> Result<Connection, String> {
    let raw = input.base_url.trim();
    if raw.is_empty() {
        return Err("authentik needs its URL (https://auth.example.com)".into());
    }
    Ok(Connection { base_url: check_site_url(raw, allow_private)?, token: String::new() })
}

fn checked_token(input: &AuthentikInput) -> Result<Option<String>, ApiError> {
    let token = input.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if token.is_some_and(|t| t.len() > MAX_TOKEN_CHARS || t.chars().any(char::is_control)) {
        return Err(ApiError::BadRequest("The token is not valid".into()));
    }
    Ok(token.map(str::to_string))
}

/// The token to use: the one entered, or the saved one when the URL didn't change.
async fn token_for(st: &AppState, org: &str, input: &AuthentikInput, base_url: &str) -> Result<String, ApiError> {
    if let Some(t) = checked_token(input)? {
        return Ok(t);
    }
    match crate::authentik::crawler::connection(st, org).await? {
        Some((_, saved)) if saved.base_url == base_url => Ok(saved.token),
        Some(_) => Err(ApiError::BadRequest("Enter the token again: the authentik URL changed".into())),
        None => Err(ApiError::BadRequest("authentik needs an API token".into())),
    }
}

/// Tests a connection: the token's user, and whether it can read events.
pub async fn check(api: &dyn AuthentikApi, conn: &Connection) -> AuthentikTest {
    let failed = |message: String, user: Option<String>| AuthentikTest { ok: false, message, user, events: None };
    let user = match api.get(&conn.base_url, &conn.token, &ak::me_path()).await {
        Ok(me) => me.pointer("/user/username").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
        Err(e @ AuthentikError::NotFound(_)) => return failed(format!("No authentik API at this URL: {}", e.message()), None),
        Err(e) => return failed(format!("authentik refused the token: {}", e.message()), None),
    };
    match api.get(&conn.base_url, &conn.token, &ak::events_path(1)).await {
        Ok(page) => {
            let events = page.pointer("/pagination/count").and_then(|c| c.as_u64()).unwrap_or(0);
            AuthentikTest { ok: true, message: format!("Connected as {user}: {events} events to read"), user: Some(user), events: Some(events) }
        }
        Err(AuthentikError::Auth(_)) => failed(format!("{user} can't view events: give its role the authentik_events.view_event permission"), Some(user)),
        Err(e) => failed(format!("Reading the events failed: {}", e.message()), Some(user)),
    }
}

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, AuthentikConnection)>, sqlx::Error> {
    let row: Option<(String, String, i64)> = sqlx::query_as("SELECT id::text, base_url, (extract(epoch FROM updated_at) * 1000)::bigint FROM authentik_connections WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(|(id, base_url, updated_at)| (id, AuthentikConnection { base_url, updated_at })))
}

/// The org's connection and its sync status.
pub async fn status(st: &AppState, org: &str) -> Result<AuthentikStatus, sqlx::Error> {
    let mut out = AuthentikStatus { connection: None, last_sync_at: None, synced_until: None, events_last_sync: 0, error: None };
    let Some((id, connection)) = saved(st, org).await? else { return Ok(out) };
    out.connection = Some(connection);
    if let Some(s) = jobs::status(st, PROVIDER, org, &id).await?.into_iter().find(|s| s.scope == SCOPE) {
        (out.last_sync_at, out.synced_until, out.events_last_sync, out.error) = (s.last_success_at, s.collected_until, s.metrics, s.error);
    }
    Ok(out)
}

/// Asks the poller to run the org's job now.
async fn poll_now(st: &AppState, org: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&st.postgres).await?.rows_affected())
}

/// The authentik connection and its sync status (no token).
#[utoipa::path(get, path = "/api/v1/integrations/authentik", operation_id = "authentik_status", responses((status = 200, body = AuthentikStatus)), tag = "integrations")]
pub async fn get_authentik(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<AuthentikStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the authentik connection (admins). The token is sealed; leave it out to keep the saved one (only while
/// the URL stays the same).
#[utoipa::path(put, path = "/api/v1/integrations/authentik", operation_id = "authentik_save", request_body = AuthentikInput,
    responses((status = 200, body = AuthentikStatus), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_authentik(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<AuthentikInput>) -> ApiResult<AuthentikStatus> {
    let conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    let previous = saved(&st, &ctx.org_id).await?;
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO authentik_connections (org_id, base_url, token_enc, updated_by) VALUES ($1, $2, $3, $4::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET base_url = EXCLUDED.base_url, token_enc = EXCLUDED.token_enc, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&conn.base_url)
    .bind(st.secrets.encrypt(&token))
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    // Another instance: its events start from the first sync's lookback again.
    if previous.as_ref().is_some_and(|(_, p)| p.base_url != conn.base_url) {
        sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    }
    // The job now, rather than at the poller's next planning (which keeps it).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, org_id, id, $3 FROM authentik_connections WHERE org_id = $2 ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind(SCOPE)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    poll_now(&st, &ctx.org_id).await?;
    let detail = format!("{}{}", conn.base_url, if input.token.as_deref().is_some_and(|t| !t.trim().is_empty()) { ", token replaced" } else { "" });
    ctx.audit(&st, "integrations.authentik_saved", "authentik", &detail).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the authentik connection, its token and its polling (admins). Stored logs stay.
#[utoipa::path(delete, path = "/api/v1/integrations/authentik", operation_id = "authentik_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_authentik(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("DELETE FROM authentik_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("authentik is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integrations.authentik_deleted", "authentik", "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): the token's user and whether it can read events.
#[utoipa::path(post, path = "/api/v1/integrations/authentik/test", operation_id = "authentik_test", request_body = AuthentikInput,
    responses((status = 200, body = AuthentikTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_authentik(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<AuthentikInput>) -> ApiResult<AuthentikTest> {
    let mut conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    conn.token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    Ok(Json(check(&Rest { allow_private: st.config.portal_allow_private }, &conn).await))
}

/// Polls authentik now instead of within the minute (admins).
#[utoipa::path(post, path = "/api/v1/integrations/authentik/sync", operation_id = "authentik_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_authentik(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("authentik is not connected".into()));
    }
    poll_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentik::fixtures::{self, FakeAuthentik};
    use serde_json::json;

    fn input(v: serde_json::Value) -> AuthentikInput {
        serde_json::from_value(v).unwrap()
    }

    fn conn() -> Connection {
        Connection { base_url: "https://portal.example.com".into(), token: "ak-test-token".into() }
    }

    #[test]
    fn the_url_is_validated_with_the_portal_url_rules() {
        assert_eq!(validate(&input(json!({"baseUrl": " https://portal.example.com/ "})), false).unwrap().base_url, "https://portal.example.com");
        for (body, needle) in [
            (json!({}), "URL"),
            (json!({"baseUrl": "http://portal.example.com"}), "https"),
            (json!({"baseUrl": "https://user:pw@portal.example.com"}), "credentials"),
            (json!({"baseUrl": "https://10.0.0.5"}), ""),
        ] {
            let err = validate(&input(body.clone()), false).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
        assert!(validate(&input(json!({"baseUrl": "http://10.0.0.5:9000"})), true).is_ok());
    }

    #[tokio::test]
    async fn testing_reports_the_user_and_the_events_it_can_read() {
        let api = FakeAuthentik::default();
        api.answer(ak::me_path(), json!({"user": {"username": "observecraft-reader"}}));
        api.answer(ak::events_path(1), fixtures::page(vec![fixtures::login("a", "2026-11-06T11:00:00Z")], 0));
        let t = check(&api, &conn()).await;
        assert_eq!(t, AuthentikTest { ok: true, message: "Connected as observecraft-reader: 1 events to read".into(), user: Some("observecraft-reader".into()), events: Some(1) });
    }

    #[tokio::test]
    async fn a_refused_token_is_reported_without_echoing_it() {
        let t = check(&FakeAuthentik { refuse: true, ..Default::default() }, &conn()).await;
        assert!(!t.ok && t.message.starts_with("authentik refused the token"), "{t:?}");
        assert!(!serde_json::to_string(&t).unwrap().contains("ak-test-token"));
        // A URL that isn't authentik's API.
        let t = check(&FakeAuthentik::default(), &conn()).await;
        assert!(!t.ok && t.message.starts_with("No authentik API"), "{t:?}");
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_token() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "authentik-a").await;
        let b = crate::test_support::admin_of_new_org(st, "authentik-b").await;
        let token = "ak-SuperSecret-123";
        let saved_a = put_authentik(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://portal.example.com", "token": token})))).await.unwrap().0;
        assert_eq!(saved_a.connection.as_ref().unwrap().base_url, "https://portal.example.com");
        assert!(!serde_json::to_string(&saved_a).unwrap().contains(token));
        let sealed: String = sqlx::query_scalar("SELECT token_enc FROM authentik_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains(token));
        // Saving creates the events job right away.
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND next_run_at <= now()").bind(PROVIDER).bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(scopes, [SCOPE]);

        // Org B sees nothing of A's, can't delete or sync it, and its first save needs a token.
        assert!(get_authentik(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_authentik(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_authentik(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        let err = put_authentik(State(st.clone()), Admin(b.clone()), Json(input(json!({"baseUrl": "https://portal.example.com"})))).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));

        // A keeps its token while the URL stays; a new URL needs it again.
        let _ = put_authentik(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://portal.example.com/"})))).await.unwrap();
        let (id, c) = crate::authentik::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!(c.token, token);
        let err = put_authentik(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://auth.other.example.com"})))).await.unwrap_err();
        assert!(err.to_string().contains("token again") && !err.to_string().contains(token), "{err}");

        // The status reads the job.
        let job = jobs::JobKey { provider: PROVIDER.into(), org_id: a.org_id.clone(), account_id: id, scope: SCOPE.into(), collected_until: None };
        jobs::record(st, &job, &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 7, points: 7, warning: None }, 60).await.unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.synced_until, s.events_last_sync, s.error.is_none()), (Some(1_793_700_000_000), 7, true));
        assert!(s.last_sync_at.is_some());

        assert_eq!(delete_authentik(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_authentik(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
    }
}
