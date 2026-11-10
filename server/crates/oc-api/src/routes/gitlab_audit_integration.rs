//! Integrations → GitLab Audit Events: an org's connection to a GitLab instance (`gitlab_audit_connections`), whose
//! audit events the server polls into logs (`crate::gitlab_audit`): the whole instance's with an administrator's
//! token, or a group's or a project's. Admins save, test and remove it and ask for a sync; every member reads it and
//! the syncs' status.
//!
//! - The token is sealed at rest, never answered, and scrubbed from errors. A saved token is only ever sent to the
//!   URL it was saved for: changing the URL needs the token again.
//! - Requests to GitLab go through the Developer Portal connectors' client and its SSRF guard
//!   (`OC_PORTAL_ALLOW_PRIVATE` for internal instances).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::dashboards::EnabledChange;
use super::integration_installs::install_pipelines;
use super::portal_sources::check_site_url;
use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::gitlab::api::{check_path, GitlabApi, GitlabError, Rest};
use crate::gitlab_audit::api::{self as audit, Scope};
use crate::gitlab_audit::crawler::{Connection, PROVIDER, SCOPE};
use crate::state::{AppState, OrgId};

/// The catalog tile and library pipeline of the integration.
pub const INTEGRATION: &str = "gitlab-audit-events";
const MAX_TOKEN_CHARS: usize = 4096;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GitlabAuditInput {
    /// https://gitlab.com or the self-managed instance's URL.
    pub base_url: String,
    /// A personal, group or project access token with `read_api`; leave it out to keep the saved one.
    pub token: Option<String>,
    /// `instance` (an administrator's token), `group` or `project`.
    pub scope: String,
    /// The group or project path (`example-org`, `example-org/storefront-api`), for those scopes.
    pub path: String,
}

/// The connection as saved; the token is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabAuditConnection {
    pub base_url: String,
    /// `instance`, `group` or `project`.
    pub scope: String,
    /// The group or project path; empty for the instance.
    pub path: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GitlabAuditStatus {
    /// None until an admin connects GitLab.
    pub connection: Option<GitlabAuditConnection>,
    /// Epoch ms of the last successful poll.
    pub last_sync_at: Option<i64>,
    /// Epoch ms: events created up to here were read.
    pub synced_until: Option<i64>,
    /// Events sent by the last poll.
    pub events_last_sync: i32,
    /// Whether a backlog (more events than a poll reads) is being read over several polls.
    pub catching_up: bool,
    /// Why the last poll failed (token refused, not an administrator, no Premium), or its warning.
    pub error: Option<CrawlIssue>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabAuditTest {
    pub ok: bool,
    pub message: String,
    /// The token's user.
    pub user: Option<String>,
    /// Whether the user is an instance administrator.
    pub admin: Option<bool>,
}

/// The input, checked: the URL and the scope (no token).
pub fn validate(input: &GitlabAuditInput, allow_private: bool) -> Result<Connection, String> {
    let raw = input.base_url.trim();
    let base_url = check_site_url(if raw.is_empty() { "https://gitlab.com" } else { raw }, allow_private)?;
    let scope = match input.scope.trim() {
        "" | "instance" => Scope::Instance,
        "group" => Scope::Group(check_path(&input.path).map_err(|_| "Enter the group's path (example-org or example-org/platform)".to_string())?),
        "project" => Scope::Project(check_path(&input.path).map_err(|_| "Enter the project's path (example-org/storefront-api)".to_string())?),
        other => return Err(format!("{other} is not a scope: instance, group or project")),
    };
    Ok(Connection { base_url, token: String::new(), scope })
}

fn checked_token(input: &GitlabAuditInput) -> Result<Option<String>, ApiError> {
    let token = input.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if token.is_some_and(|t| t.len() > MAX_TOKEN_CHARS || t.chars().any(char::is_control)) {
        return Err(ApiError::BadRequest("The token is not valid".into()));
    }
    Ok(token.map(str::to_string))
}

/// The token to use: the one entered, or the saved one when the URL didn't change.
async fn token_for(st: &AppState, org: &str, input: &GitlabAuditInput, base_url: &str) -> Result<String, ApiError> {
    if let Some(t) = checked_token(input)? {
        return Ok(t);
    }
    match crate::gitlab_audit::crawler::connection(st, org).await? {
        Some((_, saved, _)) if saved.base_url == base_url => Ok(saved.token),
        Some(_) => Err(ApiError::BadRequest("Enter the token again: the GitLab URL changed".into())),
        None => Err(ApiError::BadRequest("GitLab needs an access token".into())),
    }
}

/// Tests a connection: the token's user (an administrator, for the instance's events) and whether it can read the
/// scope's audit events.
pub async fn check(api: &dyn GitlabApi, conn: &Connection) -> GitlabAuditTest {
    let failed = |message: String, user: Option<String>, admin: Option<bool>| GitlabAuditTest { ok: false, message, user, admin };
    let (user, admin) = match api.get(&conn.base_url, &conn.token, audit::USER).await {
        Ok(me) => (me["username"].as_str().unwrap_or_default().to_string(), me["is_admin"].as_bool().unwrap_or(false)),
        Err(e @ GitlabError::NotFound(_)) => return failed(format!("No GitLab API at this URL: {}", e.message()), None, None),
        Err(e) => return failed(format!("GitLab refused the token: {}", e.message()), None, None),
    };
    if conn.scope == Scope::Instance && !admin {
        return failed(format!("{user} is not an administrator: the instance's audit events need an administrator's token (or read a group's)"), Some(user), Some(false));
    }
    let what = match &conn.scope {
        Scope::Instance => "the instance".to_string(),
        Scope::Group(p) => format!("group {p}"),
        Scope::Project(p) => format!("project {p}"),
    };
    match api.get(&conn.base_url, &conn.token, &audit::latest_path(&conn.scope)).await {
        Ok(page) if page.is_array() => GitlabAuditTest { ok: true, message: format!("Connected as {user}: reading the audit events of {what}"), user: Some(user), admin: Some(admin) },
        Ok(_) => failed("GitLab answered without a list of audit events".into(), Some(user), Some(admin)),
        Err(GitlabError::Auth(_)) => failed(
            format!("{user} can't read the audit events of {what}: they need GitLab Premium or Ultimate, the token the read_api scope, and its user the Owner role (group) or Maintainer (project)"),
            Some(user),
            Some(admin),
        ),
        Err(GitlabError::NotFound(_)) => failed(format!("GitLab has no {what} for this token (or no audit events API: Premium or Ultimate)"), Some(user), Some(admin)),
        Err(e) => failed(format!("Reading the audit events failed: {}", e.message()), Some(user), Some(admin)),
    }
}

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, GitlabAuditConnection, bool)>, sqlx::Error> {
    let row: Option<(String, String, String, String, i64, bool)> = sqlx::query_as(
        "SELECT id::text, base_url, scope, scope_path, (extract(epoch FROM updated_at) * 1000)::bigint, backlog_before_ms IS NOT NULL FROM gitlab_audit_connections WHERE org_id = $1",
    )
    .bind(org)
    .fetch_optional(&st.postgres)
    .await?;
    Ok(row.map(|(id, base_url, scope, path, updated_at, backlog)| (id, GitlabAuditConnection { base_url, scope, path, updated_at }, backlog)))
}

/// The org's connection and its sync's status.
pub async fn status(st: &AppState, org: &str) -> Result<GitlabAuditStatus, sqlx::Error> {
    let mut out = GitlabAuditStatus { connection: None, last_sync_at: None, synced_until: None, events_last_sync: 0, catching_up: false, error: None };
    let Some((id, connection, backlog)) = saved(st, org).await? else { return Ok(out) };
    out.connection = Some(connection);
    out.catching_up = backlog;
    if let Some(s) = jobs::status(st, PROVIDER, org, &id).await?.into_iter().find(|s| s.scope == SCOPE) {
        (out.last_sync_at, out.synced_until, out.events_last_sync, out.error) = (s.last_success_at, s.collected_until, s.metrics, s.error);
    }
    Ok(out)
}

/// Asks the poller to run the org's job now.
async fn poll_now(st: &AppState, org: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&st.postgres).await?.rows_affected())
}

/// The GitLab Audit Events connection and its sync's status (no token).
#[utoipa::path(get, path = "/api/v1/integrations/gitlab-audit-events", operation_id = "gitlab_audit_status", responses((status = 200, body = GitlabAuditStatus)), tag = "integrations")]
pub async fn get_gitlab_audit(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<GitlabAuditStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the GitLab Audit Events connection (admins). The token is sealed; leave it out to keep the saved one (only
/// while the URL stays the same). A new URL or scope reads from the first sync's lookback again.
#[utoipa::path(put, path = "/api/v1/integrations/gitlab-audit-events", operation_id = "gitlab_audit_save", request_body = GitlabAuditInput,
    responses((status = 200, body = GitlabAuditStatus), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_gitlab_audit(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<GitlabAuditInput>) -> ApiResult<GitlabAuditStatus> {
    let conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    let previous = saved(&st, &ctx.org_id).await?;
    let moved = previous.as_ref().is_some_and(|(_, p, _)| p.base_url != conn.base_url || p.scope != conn.scope.kind() || p.path != conn.scope.path());
    // The GitLab Audit Events dashboard is listed once a connection exists.
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO gitlab_audit_connections (org_id, base_url, token_enc, scope, scope_path, updated_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET base_url = EXCLUDED.base_url, token_enc = EXCLUDED.token_enc, scope = EXCLUDED.scope, scope_path = EXCLUDED.scope_path, \
         updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&conn.base_url)
    .bind(st.secrets.encrypt(&token))
    .bind(conn.scope.kind())
    .bind(conn.scope.path())
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    // Another instance or scope: its events start from the first sync's lookback again.
    if moved {
        sqlx::query("UPDATE gitlab_audit_connections SET backlog_until_ms = NULL, backlog_before_ms = NULL WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    }
    // The job now, rather than at the poller's next planning (which keeps it).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, org_id, id, $3 FROM gitlab_audit_connections WHERE org_id = $2 ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind(SCOPE)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Its library log pipeline (categories, the standard attributes of events sent another way).
    install_pipelines(&st, &ctx.org_id, INTEGRATION).await?;
    poll_now(&st, &ctx.org_id).await?;
    let scope = if conn.scope.path().is_empty() { conn.scope.kind().to_string() } else { format!("{} {}", conn.scope.kind(), conn.scope.path()) };
    let detail = format!("{} ({scope}){}", conn.base_url, if input.token.as_deref().is_some_and(|t| !t.trim().is_empty()) { ", token replaced" } else { "" });
    ctx.audit(&st, "integrations.gitlab_audit_saved", INTEGRATION, &detail).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the GitLab Audit Events connection, its token and its polling (admins). Stored logs stay.
#[utoipa::path(delete, path = "/api/v1/integrations/gitlab-audit-events", operation_id = "gitlab_audit_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_gitlab_audit(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("DELETE FROM gitlab_audit_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("GitLab Audit Events is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integrations.gitlab_audit_deleted", INTEGRATION, "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): the token's user and whether it can read the scope's audit
/// events.
#[utoipa::path(post, path = "/api/v1/integrations/gitlab-audit-events/test", operation_id = "gitlab_audit_test", request_body = GitlabAuditInput,
    responses((status = 200, body = GitlabAuditTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_gitlab_audit(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<GitlabAuditInput>) -> ApiResult<GitlabAuditTest> {
    let mut conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    conn.token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    Ok(Json(check(&Rest { allow_private: st.config.portal_allow_private }, &conn).await))
}

/// Polls GitLab now instead of within the minute (admins).
#[utoipa::path(post, path = "/api/v1/integrations/gitlab-audit-events/sync", operation_id = "gitlab_audit_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_gitlab_audit(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("GitLab Audit Events is not connected".into()));
    }
    poll_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_audit::fixtures::{self, FakeAudit};
    use serde_json::json;

    fn input(v: serde_json::Value) -> GitlabAuditInput {
        serde_json::from_value(v).unwrap()
    }

    fn conn(scope: Scope) -> Connection {
        Connection { base_url: "https://gitlab.example.com".into(), token: "glpat-audit-test".into(), scope }
    }

    #[test]
    fn the_url_and_scope_are_validated() {
        let c = validate(&input(json!({"baseUrl": " https://gitlab.example.com/ "})), false).unwrap();
        assert_eq!((c.base_url.as_str(), c.scope), ("https://gitlab.example.com", Scope::Instance));
        assert_eq!(validate(&input(json!({})), false).unwrap().base_url, "https://gitlab.com", "GitLab.com by default");
        assert_eq!(validate(&input(json!({"scope": "group", "path": "/example-org/platform/"})), false).unwrap().scope, Scope::Group("example-org/platform".into()));
        for (body, needle) in [
            (json!({"baseUrl": "http://gitlab.example.com"}), "https"),
            (json!({"baseUrl": "https://user:pw@gitlab.example.com"}), "credentials"),
            (json!({"baseUrl": "https://10.0.0.5"}), ""),
            (json!({"scope": "group"}), "group's path"),
            (json!({"scope": "project", "path": "example-org/../x"}), "project's path"),
            (json!({"scope": "everything"}), "not a scope"),
        ] {
            let err = validate(&input(body.clone()), false).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
        assert!(validate(&input(json!({"baseUrl": "http://10.0.0.5:8080"})), true).is_ok());
    }

    #[tokio::test]
    async fn testing_needs_an_administrator_for_the_instance_and_reads_the_events() {
        let api = FakeAudit::with(vec![fixtures::changed(1, "2026-11-06T10:00:00Z")]);
        *api.user.lock().unwrap() = Some(json!({"username": "audit-reader", "is_admin": false}));
        let t = check(&api, &conn(Scope::Instance)).await;
        assert!(!t.ok && t.message.contains("not an administrator"), "{t:?}");
        let t = check(&api, &conn(Scope::Group("example-org".into()))).await;
        assert_eq!(t, GitlabAuditTest { ok: true, message: "Connected as audit-reader: reading the audit events of group example-org".into(), user: Some("audit-reader".into()), admin: Some(false) });
        *api.user.lock().unwrap() = Some(json!({"username": "root", "is_admin": true}));
        assert!(check(&api, &conn(Scope::Instance)).await.ok);
    }

    #[tokio::test]
    async fn a_refused_token_is_reported_without_echoing_it() {
        let t = check(&FakeAudit { refuse: true, ..Default::default() }, &conn(Scope::Instance)).await;
        assert!(!t.ok && t.message.starts_with("GitLab refused the token"), "{t:?}");
        assert!(!serde_json::to_string(&t).unwrap().contains("glpat-audit-test"));
        let t = check(&FakeAudit::default(), &conn(Scope::Instance)).await;
        assert!(!t.ok && t.message.starts_with("No GitLab API"), "{t:?}");
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_token() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "gitlab-audit-a").await;
        let b = crate::test_support::admin_of_new_org(st, "gitlab-audit-b").await;
        let token = "glpat-SuperSecret-123";
        let body = json!({"baseUrl": "https://gitlab.example.com", "token": token, "scope": "group", "path": "example-org"});
        let saved_a = put_gitlab_audit(State(st.clone()), Admin(a.clone()), Json(input(body))).await.unwrap().0;
        let c = saved_a.connection.as_ref().unwrap();
        assert_eq!((c.base_url.as_str(), c.scope.as_str(), c.path.as_str()), ("https://gitlab.example.com", "group", "example-org"));
        assert!(!serde_json::to_string(&saved_a).unwrap().contains(token));
        let sealed: String = sqlx::query_scalar("SELECT token_enc FROM gitlab_audit_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains(token));
        // Saving creates the job right away, lists the dashboard, installs the pipeline and counts as installed.
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND next_run_at <= now()").bind(PROVIDER).bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(scopes, [SCOPE]);
        assert!(super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains(INTEGRATION));
        let pipelines: Vec<String> = sqlx::query_scalar("SELECT source FROM log_pipelines WHERE org_id = $1").bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(pipelines, [format!("integration:{INTEGRATION}")]);
        assert!(super::super::integration_installs::installed(st, &a.org_id).await.unwrap().contains(&INTEGRATION.to_string()));

        // Org B sees nothing of A's, can't delete or sync it, and its first save needs a token.
        assert!(get_gitlab_audit(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_gitlab_audit(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_gitlab_audit(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(put_gitlab_audit(State(st.clone()), Admin(b.clone()), Json(input(json!({"baseUrl": "https://gitlab.example.com"})))).await, Err(ApiError::BadRequest(_))));
        assert!(!super::super::dashboards::enabled_integrations(st, &b.org_id).await.unwrap().contains(INTEGRATION));

        // The status reads the job and the backlog; another scope starts over.
        let (id, _, _) = crate::gitlab_audit::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        let job = jobs::JobKey { provider: PROVIDER.into(), org_id: a.org_id.clone(), account_id: id.clone(), scope: SCOPE.into(), collected_until: None };
        jobs::record(st, &job, &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 7, points: 7, warning: None }, 60).await.unwrap();
        sqlx::query("UPDATE gitlab_audit_connections SET backlog_until_ms = 2, backlog_before_ms = 1 WHERE org_id = $1").bind(&a.org_id).execute(&st.postgres).await.unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.synced_until, s.events_last_sync, s.catching_up, s.error.is_none()), (Some(1_793_700_000_000), 7, true, true));
        let _ = put_gitlab_audit(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://gitlab.example.com/", "scope": "instance"})))).await.unwrap();
        let (_, c, backlog) = crate::gitlab_audit::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!((c.token.as_str(), c.scope, backlog), (token, Scope::Instance, None), "the token kept while the URL stays");
        assert_eq!(status(st, &a.org_id).await.unwrap().synced_until, None, "read again from the lookback");
        let err = put_gitlab_audit(State(st.clone()), Admin(a.clone()), Json(input(json!({"baseUrl": "https://gitlab.other.example.com"})))).await.unwrap_err();
        assert!(err.to_string().contains("token again") && !err.to_string().contains(token), "{err}");

        assert_eq!(delete_gitlab_audit(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_gitlab_audit(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
    }
}
