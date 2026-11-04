//! Software Delivery → Settings → GitLab CI: an org's connection to GitLab (`gitlab_ci_connections`),
//! which the server polls for finished pipelines (`crate::gitlab`). Admins save, test and remove it and
//! ask for a sync; every member reads it and the sync's status.
//!
//! - The token is sealed at rest, never answered, and scrubbed from errors. A saved token is only ever
//!   sent to the URL it was saved for: changing the URL needs the token again.
//! - Requests to GitLab go through the Developer Portal connectors' client and its SSRF guard
//!   (`OC_PORTAL_ALLOW_PRIVATE` for internal instances).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::portal_sources::{check_base_url, PortalProvider};
use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::error::{ApiError, ApiResult};
use crate::gitlab::api::{self as gl, check_path, GitlabApi, Rest};
use crate::gitlab::crawler::{discover, scope_project_id, Connection, PROVIDER};
use crate::state::{AppState, OrgId};

const MAX_TOKEN_CHARS: usize = 4096;
const MAX_LISTED_PROJECTS: usize = 100;

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GitlabCiInput {
    /// `https://gitlab.com` (empty) or the self-managed instance's URL.
    pub base_url: String,
    /// A personal, group or project access token with `read_api`; leave it out to keep the saved one.
    pub token: Option<String>,
    /// A group to follow (`example-org` or `example-org/platform`).
    pub group_path: String,
    pub include_subgroups: bool,
    /// Projects to follow (`example-org/storefront-api`).
    pub project_paths: Vec<String>,
}

/// The connection as saved; the token is never answered.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabCiConnection {
    pub base_url: String,
    pub group_path: String,
    pub include_subgroups: bool,
    pub project_paths: Vec<String>,
    /// Epoch ms.
    pub updated_at: i64,
}

/// A followed project's polling.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GitlabCiProject {
    pub id: u64,
    pub path: String,
    /// Epoch ms of the last successful poll.
    pub last_sync_at: Option<i64>,
    /// Epoch ms: pipelines updated up to here were read.
    pub synced_until: Option<i64>,
    pub error: Option<CrawlIssue>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GitlabCiStatus {
    /// None until an admin connects GitLab.
    pub connection: Option<GitlabCiConnection>,
    pub projects: Vec<GitlabCiProject>,
    /// Epoch ms of the last project list (discovery).
    pub discovered_at: Option<i64>,
    /// Why the project list couldn't be read (token refused, group not found, …).
    pub discovery_error: Option<CrawlIssue>,
    /// Epoch ms of the latest successful project poll.
    pub last_sync_at: Option<i64>,
    /// Pipelines and jobs exported (the last 35 days).
    pub pipelines_imported: i64,
    pub jobs_imported: i64,
    /// Epoch ms of the latest exported pipeline.
    pub last_import_at: Option<i64>,
}

/// The outcome of testing a connection.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabCiTest {
    pub ok: bool,
    pub message: String,
    /// The token's user.
    pub user: Option<String>,
    /// The token's scopes, when GitLab reports them.
    pub scopes: Vec<String>,
    /// Projects the connection would follow, and a few of their paths.
    pub projects: usize,
    pub sample: Vec<String>,
}

/// The input, checked: base URL, group and project paths (no token).
pub fn validate(input: &GitlabCiInput, allow_private: bool) -> Result<Connection, String> {
    let base_url = check_base_url(PortalProvider::Gitlab, &input.base_url, allow_private)?;
    let group_path = if input.group_path.trim().is_empty() { String::new() } else { check_path(&input.group_path)? };
    if input.project_paths.len() > MAX_LISTED_PROJECTS {
        return Err(format!("List at most {MAX_LISTED_PROJECTS} projects, or follow their group"));
    }
    let mut project_paths = Vec::new();
    for p in input.project_paths.iter().filter(|p| !p.trim().is_empty()) {
        let p = check_path(p)?;
        if !project_paths.contains(&p) {
            project_paths.push(p);
        }
    }
    if group_path.is_empty() && project_paths.is_empty() {
        return Err("Choose a group or at least one project to follow".into());
    }
    Ok(Connection { base_url, token: String::new(), group_path, include_subgroups: input.include_subgroups, project_paths })
}

fn checked_token(input: &GitlabCiInput) -> Result<Option<String>, ApiError> {
    let token = input.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if token.is_some_and(|t| t.len() > MAX_TOKEN_CHARS || t.chars().any(char::is_control)) {
        return Err(ApiError::BadRequest("The token is not valid".into()));
    }
    Ok(token.map(str::to_string))
}

/// The token to use: the one entered, or the saved one when the URL didn't change.
async fn token_for(st: &AppState, org: &str, input: &GitlabCiInput, base_url: &str) -> Result<String, ApiError> {
    if let Some(t) = checked_token(input)? {
        return Ok(t);
    }
    match crate::gitlab::crawler::connection(st, org).await? {
        Some((_, saved)) if saved.base_url == base_url => Ok(saved.token),
        Some(_) => Err(ApiError::BadRequest("Enter the token again: the GitLab URL changed".into())),
        None => Err(ApiError::BadRequest("GitLab needs an access token with the read_api scope".into())),
    }
}

/// Tests a connection: the token's user and scopes, and the projects it would follow.
pub async fn check(api: &dyn GitlabApi, conn: &Connection) -> GitlabCiTest {
    let failed = |message: String| GitlabCiTest { ok: false, message, user: None, scopes: vec![], projects: 0, sample: vec![] };
    let user = match api.get(&conn.base_url, &conn.token, &gl::user_path()).await {
        Ok(u) => u["username"].as_str().unwrap_or_default().to_string(),
        Err(e) => return failed(format!("GitLab refused the token: {}", e.message())),
    };
    // Older GitLab versions and some token kinds don't answer this: the scopes are then unknown.
    let scopes: Vec<String> = match api.get(&conn.base_url, &conn.token, &gl::token_path()).await {
        Ok(t) => t["scopes"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect(),
        Err(_) => vec![],
    };
    if !scopes.is_empty() && !scopes.iter().any(|s| s == "read_api" || s == "api") {
        return GitlabCiTest { user: Some(user), scopes, ..failed("The token needs the read_api scope".into()) };
    }
    match discover(api, conn).await {
        Ok(projects) if projects.is_empty() => GitlabCiTest { user: Some(user), scopes, ..failed("The token sees no project in this group".into()) },
        Ok(projects) => GitlabCiTest {
            ok: true,
            message: format!("Connected as {user}: {} project{} to follow", projects.len(), if projects.len() == 1 { "" } else { "s" }),
            user: Some(user),
            scopes,
            projects: projects.len(),
            sample: projects.iter().take(5).map(|(_, p)| p.clone()).collect(),
        },
        Err(e) => GitlabCiTest { user: Some(user), scopes, ..failed(format!("Listing the projects failed: {}", e.message())) },
    }
}

async fn saved(st: &AppState, org: &str) -> Result<Option<(String, GitlabCiConnection)>, sqlx::Error> {
    let row: Option<(String, String, String, bool, Vec<String>, i64)> = sqlx::query_as(
        "SELECT id::text, base_url, group_path, include_subgroups, project_paths, (extract(epoch FROM updated_at) * 1000)::bigint FROM gitlab_ci_connections WHERE org_id = $1",
    )
    .bind(org)
    .fetch_optional(&st.postgres)
    .await?;
    Ok(row.map(|(id, base_url, group_path, include_subgroups, project_paths, updated_at)| (id, GitlabCiConnection { base_url, group_path, include_subgroups, project_paths, updated_at })))
}

/// The org's connection and its sync status.
pub async fn status(st: &AppState, org: &str) -> Result<GitlabCiStatus, sqlx::Error> {
    let Some((id, connection)) = saved(st, org).await? else {
        return Ok(GitlabCiStatus { connection: None, projects: vec![], discovered_at: None, discovery_error: None, last_sync_at: None, pipelines_imported: 0, jobs_imported: 0, last_import_at: None });
    };
    let scopes = jobs::status(st, PROVIDER, org, &id).await?;
    let (pipelines_imported, jobs_imported, last_import_at): (i64, i64, Option<i64>) =
        sqlx::query_as("SELECT count(*), COALESCE(sum(jobs), 0)::bigint, (extract(epoch FROM max(exported_at)) * 1000)::bigint FROM gitlab_ci_exported WHERE org_id = $1")
            .bind(org)
            .fetch_one(&st.postgres)
            .await?;
    let mut out = GitlabCiStatus { connection: Some(connection), projects: vec![], discovered_at: None, discovery_error: None, last_sync_at: None, pipelines_imported, jobs_imported, last_import_at };
    for s in scopes {
        if s.scope.is_empty() {
            out.discovered_at = s.last_success_at;
            out.discovery_error = s.error;
        } else if let Some(pid) = scope_project_id(&s.scope) {
            out.last_sync_at = out.last_sync_at.max(s.last_success_at);
            let path = s.scope.split_once(':').map(|(_, p)| p.to_string()).unwrap_or_default();
            out.projects.push(GitlabCiProject { id: pid, path, last_sync_at: s.last_success_at, synced_until: s.collected_until, error: s.error });
        }
    }
    out.projects.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Asks the poller to run the org's jobs now (the discovery first).
async fn poll_now(st: &AppState, org: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(org).execute(&st.postgres).await?.rows_affected())
}

/// The GitLab CI connection and its sync status (no token).
#[utoipa::path(get, path = "/api/v1/ci/gitlab", operation_id = "ci_gitlab_status", responses((status = 200, body = GitlabCiStatus)), tag = "ci")]
pub async fn get_gitlab(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<GitlabCiStatus> {
    Ok(Json(status(&st, &org).await?))
}

/// Saves the GitLab CI connection (admins). The token is sealed; leave it out to keep the saved one
/// (only while the URL stays the same).
#[utoipa::path(put, path = "/api/v1/ci/gitlab", operation_id = "ci_gitlab_save", request_body = GitlabCiInput,
    responses((status = 200, body = GitlabCiStatus), (status = 400, body = crate::error::ErrorBody)), tag = "ci")]
pub async fn put_gitlab(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<GitlabCiInput>) -> ApiResult<GitlabCiStatus> {
    let conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    let previous = saved(&st, &ctx.org_id).await?;
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO gitlab_ci_connections (org_id, base_url, token_enc, group_path, include_subgroups, project_paths, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7::uuid) \
         ON CONFLICT (org_id) DO UPDATE SET base_url = EXCLUDED.base_url, token_enc = EXCLUDED.token_enc, group_path = EXCLUDED.group_path, \
         include_subgroups = EXCLUDED.include_subgroups, project_paths = EXCLUDED.project_paths, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&conn.base_url)
    .bind(st.secrets.encrypt(&token))
    .bind(&conn.group_path)
    .bind(conn.include_subgroups)
    .bind(&conn.project_paths)
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    // Another GitLab: its project and pipeline ids are not the old one's.
    if previous.as_ref().is_some_and(|(_, p)| p.base_url != conn.base_url) {
        sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM gitlab_ci_exported WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    }
    // The discovery job now, rather than at the poller's next planning (which keeps it).
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, org_id, id, '' FROM gitlab_ci_connections WHERE org_id = $2 ON CONFLICT DO NOTHING")
        .bind(PROVIDER)
        .bind(&ctx.org_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    poll_now(&st, &ctx.org_id).await?;
    let detail = format!("{}{}", conn.base_url, if input.token.as_deref().is_some_and(|t| !t.trim().is_empty()) { ", token replaced" } else { "" });
    ctx.audit(&st, "ci.gitlab_saved", "gitlab", &detail).await;
    Ok(Json(status(&st, &ctx.org_id).await?))
}

/// Removes the GitLab CI connection, its token and its polling (admins). Imported pipelines stay.
#[utoipa::path(delete, path = "/api/v1/ci/gitlab", operation_id = "ci_gitlab_delete", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "ci")]
pub async fn delete_gitlab(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("DELETE FROM gitlab_ci_connections WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("GitLab is not connected".into()));
    }
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2").bind(PROVIDER).bind(&ctx.org_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM gitlab_ci_exported WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "ci.gitlab_deleted", "gitlab", "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): the token's user and scopes, and the projects found.
#[utoipa::path(post, path = "/api/v1/ci/gitlab/test", operation_id = "ci_gitlab_test", request_body = GitlabCiInput,
    responses((status = 200, body = GitlabCiTest), (status = 400, body = crate::error::ErrorBody)), tag = "ci")]
pub async fn test_gitlab(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<GitlabCiInput>) -> ApiResult<GitlabCiTest> {
    let mut conn = validate(&input, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    conn.token = token_for(&st, &ctx.org_id, &input, &conn.base_url).await?;
    Ok(Json(check(&Rest { allow_private: st.config.portal_allow_private }, &conn).await))
}

/// Polls GitLab now instead of within the minute (admins).
#[utoipa::path(post, path = "/api/v1/ci/gitlab/sync", operation_id = "ci_gitlab_sync", responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "ci")]
pub async fn sync_gitlab(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    if saved(&st, &ctx.org_id).await?.is_none() {
        return Err(ApiError::NotFound("GitLab is not connected".into()));
    }
    poll_now(&st, &ctx.org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::fixtures::{self, FakeGitlab};
    use serde_json::json;

    fn input(v: serde_json::Value) -> GitlabCiInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_validated_with_the_portal_url_rules() {
        let c = validate(&input(json!({"baseUrl": "", "groupPath": "/example-org/", "includeSubgroups": true, "projectPaths": ["other/billing", "other/billing", " "]})), false).unwrap();
        assert_eq!((c.base_url.as_str(), c.group_path.as_str(), c.project_paths.clone()), ("https://gitlab.com", "example-org", vec!["other/billing".to_string()]));
        let self_managed = validate(&input(json!({"baseUrl": "https://gitlab.example.com/", "projectPaths": ["example-org/storefront-api"]})), false).unwrap();
        assert_eq!(self_managed.base_url, "https://gitlab.example.com");
        for (body, needle) in [
            (json!({"baseUrl": "http://gitlab.example.com", "groupPath": "g"}), "https"),
            (json!({"baseUrl": "https://user:pw@gitlab.example.com", "groupPath": "g"}), "credentials"),
            (json!({"baseUrl": "https://10.0.0.5", "groupPath": "g"}), ""),
            (json!({"groupPath": "../admin"}), "path"),
            (json!({}), "group or at least one project"),
        ] {
            let err = validate(&input(body.clone()), false).unwrap_err();
            assert!(err.contains(needle), "{body}: {err}");
        }
        // Internal instances need the operator's flag.
        assert!(validate(&input(json!({"baseUrl": "http://10.0.0.5", "groupPath": "g"})), true).is_ok());
    }

    fn conn() -> Connection {
        Connection { base_url: "https://gitlab.example.com".into(), token: "glpat-test".into(), group_path: String::new(), include_subgroups: true, project_paths: vec!["example-org/storefront-api".into()] }
    }

    #[tokio::test]
    async fn testing_reports_the_user_scopes_and_projects() {
        let g = FakeGitlab::default();
        g.answer(gl::user_path(), json!({"username": "ci-reader"}));
        g.answer(gl::token_path(), json!({"scopes": ["read_api"]}));
        g.answer(gl::project_path("example-org/storefront-api"), fixtures::project());
        let t = check(&g, &conn()).await;
        assert_eq!(t, GitlabCiTest { ok: true, message: "Connected as ci-reader: 1 project to follow".into(), user: Some("ci-reader".into()), scopes: vec!["read_api".into()], projects: 1, sample: vec!["example-org/storefront-api".into()] });

        g.answer(gl::token_path(), json!({"scopes": ["read_user"]}));
        let t = check(&g, &conn()).await;
        assert!(!t.ok && t.message.contains("read_api"), "{t:?}");
    }

    #[tokio::test]
    async fn a_refused_token_is_reported_without_echoing_it() {
        let t = check(&FakeGitlab::default(), &conn()).await;
        assert!(!t.ok && t.message.starts_with("GitLab refused the token"), "{t:?}");
        assert!(!serde_json::to_string(&t).unwrap().contains("glpat-test"));
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_the_token() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "gitlab-a").await;
        let b = crate::test_support::admin_of_new_org(st, "gitlab-b").await;
        let token = "glpat-SuperSecret-123";
        let body = input(json!({"baseUrl": "https://gitlab.example.com", "token": token, "groupPath": "example-org", "includeSubgroups": true}));
        let saved_a = put_gitlab(State(st.clone()), Admin(a.clone()), Json(body)).await.unwrap().0;
        assert_eq!(saved_a.connection.as_ref().unwrap().group_path, "example-org");
        assert!(!serde_json::to_string(&saved_a).unwrap().contains(token));
        let sealed: String = sqlx::query_scalar("SELECT token_enc FROM gitlab_ci_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains(token));

        // Org B sees nothing of A's, can't delete or sync it.
        assert!(get_gitlab(State(st.clone()), OrgId(b.org_id.clone())).await.unwrap().0.connection.is_none());
        assert!(matches!(delete_gitlab(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_gitlab(State(st.clone()), Admin(b.clone())).await, Err(ApiError::NotFound(_))));
        // B's first save needs a token: A's is never used.
        let err = put_gitlab(State(st.clone()), Admin(b.clone()), Json(input(json!({"baseUrl": "https://gitlab.example.com", "groupPath": "example-org"})))).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));

        // A keeps its token while the URL stays; a new URL needs it again.
        let keep = input(json!({"baseUrl": "https://gitlab.example.com", "groupPath": "example-org", "projectPaths": ["other/billing"]}));
        let _ = put_gitlab(State(st.clone()), Admin(a.clone()), Json(keep)).await.unwrap();
        let (_, c) = crate::gitlab::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        assert_eq!((c.token.as_str(), c.project_paths.clone()), (token, vec!["other/billing".to_string()]));
        let moved = input(json!({"baseUrl": "https://gitlab.other.example.com", "groupPath": "example-org"}));
        let err = put_gitlab(State(st.clone()), Admin(a.clone()), Json(moved)).await.unwrap_err();
        assert!(err.to_string().contains("token again") && !err.to_string().contains(token), "{err}");

        assert!(get_gitlab(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_some());
        assert_eq!(delete_gitlab(State(st.clone()), Admin(a.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        assert!(get_gitlab(State(st.clone()), OrgId(a.org_id.clone())).await.unwrap().0.connection.is_none());
    }

    #[tokio::test]
    async fn status_reads_the_jobs_and_the_exported_pipelines() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "gitlab-status").await;
        let _ = put_gitlab(State(st.clone()), Admin(a.clone()), Json(input(json!({"token": "glpat-x", "groupPath": "example-org"})))).await.unwrap();
        let (id, _) = crate::gitlab::crawler::connection(st, &a.org_id).await.unwrap().unwrap();
        let discovery = jobs::JobKey { provider: PROVIDER.into(), org_id: a.org_id.clone(), account_id: id.clone(), scope: String::new(), collected_until: None };
        // Saving creates the discovery job right away (no wait for the poller's planning).
        let due: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid AND next_run_at <= now()")
            .bind(PROVIDER)
            .bind(&a.org_id)
            .bind(&id)
            .fetch_all(&st.postgres)
            .await
            .unwrap();
        assert_eq!(due, [""]);
        jobs::record(st, &discovery, &jobs::Outcome::Discovered(vec!["4211:example-org/storefront-api".into()]), 60).await.unwrap();
        let project = jobs::JobKey { scope: "4211:example-org/storefront-api".into(), ..discovery };
        jobs::record(st, &project, &jobs::Outcome::Collected { end_ms: 1_793_700_000_000, metrics: 1, points: 3, warning: None }, 60).await.unwrap();
        sqlx::query("INSERT INTO gitlab_ci_exported (org_id, project_id, pipeline_id, jobs) VALUES ($1, 4211, 55001, 3)").bind(&a.org_id).execute(&st.postgres).await.unwrap();

        let s = status(st, &a.org_id).await.unwrap();
        assert_eq!((s.pipelines_imported, s.jobs_imported, s.projects.len()), (1, 3, 1));
        assert_eq!((s.projects[0].id, s.projects[0].path.as_str(), s.projects[0].synced_until), (4211, "example-org/storefront-api", Some(1_793_700_000_000)));
        assert!(s.discovered_at.is_some() && s.last_sync_at.is_some() && s.discovery_error.is_none());
        // Another org's status is empty.
        let other = crate::test_support::new_org(st, "gitlab-status-other").await;
        assert_eq!(status(st, &other).await.unwrap().pipelines_imported, 0);
    }
}
