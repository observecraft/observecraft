//! Integrations → Google Workspace: an org's connection to a Google Workspace account
//! (`google_workspace_connections`), whose Reports API activities the server polls into logs
//! (`crate::google_workspace`): a service account's JSON key, the administrator it acts as (domain-wide delegation for
//! the read-only audit scope) and the applications to read. Admins save, test and remove it and ask for a sync; every
//! member reads it and each application's sync status.
//!
//! - The key is sealed at rest, never answered (only its service account's email), and kept when a save leaves it
//!   out.
//! - Google is called only at its own fixed hosts (`crate::google_workspace::api`), never at a URL from the key.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::dashboards::EnabledChange;
use super::integration_installs::install_pipelines;
use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::google_workspace::api::{self as gws, Credentials, WorkspaceApi, WorkspaceError};
use crate::google_workspace::crawler::PROVIDER;
use crate::state::{AppState, OrgId};

/// The catalog tile and library pipeline of the integration.
pub const INTEGRATION: &str = "google-workspace";
const MAX_KEY_CHARS: usize = 16_384;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleWorkspaceInput {
    /// The service account's JSON key file; leave it out to keep the saved one.
    pub service_account_key: Option<String>,
    /// The administrator the service account acts as (domain-wide delegation).
    pub subject: String,
    /// Reports API applications to read (`login`, `admin`, `drive`, …); empty: the defaults.
    pub applications: Vec<String>,
}

/// The connection as saved; the key is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWorkspaceConnection {
    /// The service account (the key's `client_email`).
    pub client_email: String,
    pub subject: String,
    pub applications: Vec<String>,
    /// Epoch ms.
    pub updated_at: i64,
}

/// A Reports API application the integration can read.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWorkspaceApplication {
    /// The Reports API's `applicationName` (the logs' `service`).
    pub id: String,
    pub name: String,
    /// Read when an admin picks none.
    pub default: bool,
}

/// An application's sync.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWorkspaceSync {
    pub application: String,
    /// Epoch ms of the last successful poll.
    pub last_sync_at: Option<i64>,
    /// Epoch ms: activities up to here were read.
    pub synced_until: Option<i64>,
    /// Events sent by the last poll.
    pub events_last_sync: i32,
    /// Whether a backlog (more activities than a poll reads) is being read over several polls.
    pub catching_up: bool,
    /// Why the last poll failed (delegation refused, not an administrator), or its warning.
    pub error: Option<CrawlIssue>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWorkspaceStatus {
    /// None until an admin connects Google Workspace.
    pub connection: Option<GoogleWorkspaceConnection>,
    /// Each selected application's sync.
    pub syncs: Vec<GoogleWorkspaceSync>,
    /// The applications that can be read.
    pub available: Vec<GoogleWorkspaceApplication>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWorkspaceTest {
    pub ok: bool,
    pub message: String,
    /// The service account the key belongs to.
    pub client_email: Option<String>,
}

fn available() -> Vec<GoogleWorkspaceApplication> {
    gws::APPLICATIONS.iter().map(|(id, name)| GoogleWorkspaceApplication { id: id.to_string(), name: name.to_string(), default: gws::DEFAULT_APPLICATIONS.contains(id) }).collect()
}

/// The administrator's email and the applications, checked (no key).
pub fn validate(input: &GoogleWorkspaceInput) -> Result<(String, Vec<String>), String> {
    let subject = input.subject.trim().to_lowercase();
    let ok = subject.len() <= 254
        && subject.split_once('@').is_some_and(|(u, d)| !u.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
        && !subject.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err("Enter the email of a Google Workspace administrator for the service account to act as".into());
    }
    let mut apps: Vec<String> = input.applications.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    if let Some(bad) = apps.iter().find(|a| !gws::is_application(a)) {
        return Err(format!("{bad} is not a Reports API application this integration reads"));
    }
    if apps.is_empty() {
        apps = gws::DEFAULT_APPLICATIONS.iter().map(|a| a.to_string()).collect();
    }
    apps.sort();
    apps.dedup();
    Ok((subject, apps))
}

/// The key to use: the one entered (checked), or the saved one.
async fn key_for(st: &AppState, org: &str, input: &GoogleWorkspaceInput) -> Result<gws::ServiceAccount, ApiError> {
    if let Some(k) = input.service_account_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        if k.len() > MAX_KEY_CHARS {
            return Err(ApiError::BadRequest("The key is too large for a service account key".into()));
        }
        return gws::parse_key(k).map_err(ApiError::BadRequest);
    }
    match crate::google_workspace::crawler::connection(st, org).await? {
        Some((_, saved, _)) => Ok(saved.key),
        None => Err(ApiError::BadRequest("Google Workspace needs the service account's JSON key".into())),
    }
}

/// Tests a connection: a token for the administrator, then the newest activity of each application.
pub async fn check(api: &dyn WorkspaceApi, creds: &Credentials, applications: &[String], now_ms: i64) -> GoogleWorkspaceTest {
    let client_email = Some(creds.key.client_email.clone());
    for app in applications {
        let path = gws::activities_path(app, now_ms - 86_400_000, now_ms, None, 1);
        if let Err(e) = api.get(creds, &path).await {
            let why = match &e {
                WorkspaceError::Denied(m) | WorkspaceError::Auth(m) => m.clone(),
                other => format!("Reading the {app} activities failed: {}", other.message()),
            };
            return GoogleWorkspaceTest { ok: false, message: why, client_email };
        }
    }
    GoogleWorkspaceTest { ok: true, message: format!("Connected: {} reads the activities of {} as {}", creds.key.client_email, applications.join(", "), creds.subject), client_email }
}

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, GoogleWorkspaceConnection)>, sqlx::Error> {
    let row: Option<(String, String, String, Vec<String>, i64)> =
        sqlx::query_as("SELECT id::text, client_email, subject, applications, (extract(epoch FROM updated_at) * 1000)::bigint FROM google_workspace_connections WHERE org_id = $1")
            .bind(org)
            .fetch_optional(&st.postgres)
            .await?;
    Ok(row.map(|(id, client_email, subject, applications, updated_at)| (id, GoogleWorkspaceConnection { client_email, subject, applications, updated_at })))
}

/// The org's connection and its applications' syncs.
pub async fn status(st: &AppState, org: &str) -> Result<GoogleWorkspaceStatus, sqlx::Error> {
    let mut out = GoogleWorkspaceStatus { connection: None, syncs: vec![], available: available() };
    let Some((id, connection)) = saved(st, org).await? else { return Ok(out) };
    let jobs = jobs::status(st, PROVIDER, org, &id).await?;
    let backlogs: Vec<String> = sqlx::query_scalar("SELECT application FROM google_workspace_progress WHERE org_id = $1 AND connection_id = $2::uuid AND backlog_before_ms IS NOT NULL")
        .bind(org)
        .bind(&id)
        .fetch_all(&st.postgres)
        .await?;
    for app in &connection.applications {
        let mut s = GoogleWorkspaceSync { application: app.clone(), last_sync_at: None, synced_until: None, events_last_sync: 0, catching_up: backlogs.contains(app), error: None };
        if let Some(j) = jobs.iter().find(|j| j.scope == *app) {
            (s.last_sync_at, s.synced_until, s.events_last_sync) = (j.last_success_at, j.collected_until, j.metrics);
            s.error = j.error.as_ref().map(|e| CrawlIssue { kind: e.kind.clone(), message: e.message.clone(), missing_action: e.missing_action.clone(), at: e.at });
        }
        out.syncs.push(s);
    }
    out.connection = Some(connection);
    Ok(out)
}

/// Asks the poller to run the org's jobs now.
async fn poll_now(st: &AppState, org: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&st.postgres).await?.rows_affected())
}

/// The Google Workspace connection and its applications' syncs (no key).
#[utoipa::path(get, path = "/api/v1/integrations/google-workspace", operation_id = "google_workspace_status", responses((status = 200, body = GoogleWorkspaceStatus)), tag = "integrations")]
pub async fn get_google_workspace(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<GoogleWorkspaceStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the Google Workspace connection (admins). The key is sealed; leave it out to keep the saved one. Another
/// service account or administrator reads from the first sync's lookback again; applications no longer selected stop.
#[utoipa::path(put, path = "/api/v1/integrations/google-workspace", operation_id = "google_workspace_save", request_body = GoogleWorkspaceInput,
    responses((status = 200, body = GoogleWorkspaceStatus), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn put_google_workspace(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<GoogleWorkspaceInput>) -> ApiResult<GoogleWorkspaceStatus> {
    let (subject, applications) = validate(&input).map_err(ApiError::BadRequest)?;
    let key = key_for(&st, &ctx.org_id, &input).await?;
    let previous = saved(&st, &ctx.org_id).await?;
    let moved = previous.as_ref().is_some_and(|(_, p)| p.client_email != key.client_email || p.subject != subject);
    // The Google Workspace dashboard is listed once a connection exists.
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    let id: String = sqlx::query_scalar(
        "INSERT INTO google_workspace_connections (org_id, client_email, key_enc, subject, applications, updated_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET client_email = EXCLUDED.client_email, key_enc = EXCLUDED.key_enc, subject = EXCLUDED.subject, applications = EXCLUDED.applications, \
         updated_by = EXCLUDED.updated_by, updated_at = now() RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&key.client_email)
    .bind(st.secrets.encrypt(&key.json))
    .bind(&subject)
    .bind(&applications)
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await?;
    // Another account or administrator: every application starts from the first sync's lookback again. Otherwise only
    // the applications no longer read are dropped.
    let dropped: Vec<String> =
        if moved { gws::APPLICATIONS.iter().map(|(a, _)| a.to_string()).collect() } else { gws::APPLICATIONS.iter().map(|(a, _)| a.to_string()).filter(|a| !applications.contains(a)).collect() };
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND scope = ANY($3)").bind(PROVIDER).bind(&ctx.org_id).bind(&dropped).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM google_workspace_progress WHERE org_id = $1 AND connection_id = $2::uuid AND application = ANY($3)").bind(&ctx.org_id).bind(&id).bind(&dropped).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM google_workspace_seen WHERE org_id = $1 AND connection_id = $2::uuid AND application = ANY($3)").bind(&ctx.org_id).bind(&id).bind(&dropped).execute(&mut *tx).await?;
    // The jobs now, rather than at the poller's next planning (which keeps them).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, $2, $3::uuid, a FROM unnest($4::text[]) AS a ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&applications)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Its library log pipeline (categories, statuses, the standard attributes of activities sent another way).
    install_pipelines(&st, &ctx.org_id, INTEGRATION).await?;
    poll_now(&st, &ctx.org_id).await?;
    let replaced = if input.service_account_key.as_deref().is_some_and(|k| !k.trim().is_empty()) { ", key replaced" } else { "" };
    ctx.audit(&st, "integrations.google_workspace_saved", INTEGRATION, &format!("{} as {subject} ({}){replaced}", key.client_email, applications.join(", "))).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the Google Workspace connection, its key and its polling (admins). Stored logs stay.
#[utoipa::path(delete, path = "/api/v1/integrations/google-workspace", operation_id = "google_workspace_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_google_workspace(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    // Its progress and seen activities go with it (ON DELETE CASCADE).
    let done = sqlx::query("DELETE FROM google_workspace_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Google Workspace is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integrations.google_workspace_deleted", INTEGRATION, "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): a token for the administrator and each application's
/// activities.
#[utoipa::path(post, path = "/api/v1/integrations/google-workspace/test", operation_id = "google_workspace_test", request_body = GoogleWorkspaceInput,
    responses((status = 200, body = GoogleWorkspaceTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_google_workspace(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<GoogleWorkspaceInput>) -> ApiResult<GoogleWorkspaceTest> {
    let (subject, applications) = validate(&input).map_err(ApiError::BadRequest)?;
    let key = key_for(&st, &ctx.org_id, &input).await?;
    let creds = Credentials { key, subject };
    Ok(Json(check(&gws::Rest::default(), &creds, &applications, chrono::Utc::now().timestamp_millis()).await))
}

/// Polls Google now instead of within five minutes (admins).
#[utoipa::path(post, path = "/api/v1/integrations/google-workspace/sync", operation_id = "google_workspace_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_google_workspace(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("Google Workspace is not connected".into()));
    }
    poll_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google_workspace::fixtures::{self, FakeReports};
    use serde_json::json;

    fn input(v: serde_json::Value) -> GoogleWorkspaceInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_administrator_and_applications_are_validated() {
        let (subject, apps) = validate(&input(json!({"subject": " Admin@Example.com "}))).unwrap();
        assert_eq!(subject, "admin@example.com");
        let mut defaults: Vec<String> = gws::DEFAULT_APPLICATIONS.iter().map(|a| a.to_string()).collect();
        defaults.sort();
        assert_eq!(apps, defaults, "the defaults when none is picked");
        assert_eq!(validate(&input(json!({"subject": "admin@example.com", "applications": ["drive", "login", "drive"]}))).unwrap().1, ["drive", "login"]);
        for (body, needle) in [
            (json!({"subject": ""}), "administrator"),
            (json!({"subject": "admin"}), "administrator"),
            (json!({"subject": "admin@example"}), "administrator"),
            (json!({"subject": "ad min@example.com"}), "administrator"),
            (json!({"subject": "admin@example.com", "applications": ["gmail/../x"]}), "not a Reports API application"),
        ] {
            let err = validate(&input(body.clone())).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
    }

    #[tokio::test]
    async fn testing_reads_each_application_and_reports_a_refused_delegation() {
        let api = FakeReports::with(vec![fixtures::login_success("1", "2026-11-06T10:00:00Z")]);
        let apps = vec!["admin".to_string(), "login".to_string()];
        let t = check(&api, &fixtures::credentials(), &apps, 1_793_959_200_000).await;
        assert!(t.ok, "{t:?}");
        assert_eq!(t.message, "Connected: observecraft-reports@example-project.iam.gserviceaccount.com reads the activities of admin, login as admin@example.com");
        assert_eq!(api.requested().len(), 2);
        assert!(api.requested()[0].contains("/applications/admin?") && api.requested()[0].ends_with("maxResults=1"));
        let refused = FakeReports { refuse: true, ..Default::default() };
        let t = check(&refused, &fixtures::credentials(), &apps, 1_793_959_200_000).await;
        assert!(!t.ok && t.message.contains("domain-wide delegation"), "{t:?}");
        assert!(!serde_json::to_string(&t).unwrap().contains("PRIVATE"));
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_key() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "google-workspace-a").await;
        let b = crate::test_support::admin_of_new_org(st, "google-workspace-b").await;
        let key = fixtures::key_json();
        let body = json!({"serviceAccountKey": key, "subject": "admin@example.com", "applications": ["login", "admin"]});
        let saved_a = put_google_workspace(State(st.clone()), Admin(a.clone()), Json(input(body))).await.unwrap().0;
        let c = saved_a.connection.as_ref().unwrap();
        assert_eq!(
            (c.client_email.as_str(), c.subject.as_str(), c.applications.clone()),
            ("observecraft-reports@example-project.iam.gserviceaccount.com", "admin@example.com", vec!["admin".to_string(), "login".to_string()])
        );
        assert_eq!(saved_a.syncs.iter().map(|s| s.application.as_str()).collect::<Vec<_>>(), ["admin", "login"]);
        assert!(saved_a.available.iter().any(|x| x.id == "drive" && x.default));
        let answered = serde_json::to_string(&saved_a).unwrap();
        assert!(!answered.contains("PRIVATE KEY") && !answered.contains("private_key"));
        let sealed: String = sqlx::query_scalar("SELECT key_enc FROM google_workspace_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains("private_key"));
        // Saving creates the jobs right away, lists the dashboard, installs the pipeline and counts as installed.
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND next_run_at <= now() ORDER BY scope")
            .bind(PROVIDER)
            .bind(&a.org_id)
            .fetch_all(&st.postgres)
            .await
            .unwrap();
        assert_eq!(scopes, ["admin", "login"]);
        assert!(super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains(INTEGRATION));
        let pipelines: Vec<String> = sqlx::query_scalar("SELECT source FROM log_pipelines WHERE org_id = $1").bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(pipelines, [format!("integration:{INTEGRATION}")]);
        assert!(super::super::integration_installs::installed(st, &a.org_id).await.unwrap().contains(&INTEGRATION.to_string()));

        // Org B sees nothing of A's, can't delete or sync it, and its first save needs a key.
        assert!(get_google_workspace(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_google_workspace(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_google_workspace(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(put_google_workspace(State(st.clone()), Admin(b.clone()), Json(input(json!({"subject": "admin@example.com"})))).await, Err(ApiError::BadRequest(_))));
        assert!(!super::super::dashboards::enabled_integrations(st, &b.org_id).await.unwrap().contains(INTEGRATION));
        let bad =
            put_google_workspace(State(st.clone()), Admin(b.clone()), Json(input(json!({"subject": "admin@example.com", "serviceAccountKey": "{\"type\": \"authorized_user\"}"})))).await.unwrap_err();
        assert!(bad.to_string().contains("service account"), "{bad}");

        // The status reads each application's job and backlog; dropping an application stops it, the key is kept.
        let (id, _, _) = crate::google_workspace::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        let job = jobs::JobKey { provider: PROVIDER.into(), org_id: a.org_id.clone(), account_id: id.clone(), scope: "login".into(), collected_until: None };
        jobs::record(st, &job, &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 7, points: 7, warning: None }, 300).await.unwrap();
        sqlx::query("INSERT INTO google_workspace_progress (org_id, connection_id, application, backlog_start_ms, backlog_until_ms, backlog_before_ms) VALUES ($1, $2::uuid, 'login', 1, 3, 2)")
            .bind(&a.org_id)
            .bind(&id)
            .execute(&st.postgres)
            .await
            .unwrap();
        let s = status(st, &a.org_id).await.unwrap();
        let login = s.syncs.iter().find(|s| s.application == "login").unwrap();
        assert_eq!((login.synced_until, login.events_last_sync, login.catching_up, login.error.is_none()), (Some(1_793_700_000_000), 7, true, true));
        let _ = put_google_workspace(State(st.clone()), Admin(a.clone()), Json(input(json!({"subject": "admin@example.com", "applications": ["admin"]})))).await.unwrap();
        let (_, creds, apps) = crate::google_workspace::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!((creds.key.json.as_str(), apps), (key.as_str(), vec!["admin".to_string()]), "the key kept");
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(scopes, ["admin"]);
        let progress: i64 = sqlx::query_scalar("SELECT count(*) FROM google_workspace_progress WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(progress, 0, "login's progress dropped with it");

        assert_eq!(delete_google_workspace(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_google_workspace(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
    }
}
