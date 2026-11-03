//! Assist Code: coding sessions on the org's repositories (runs of kind `code` that read the code and
//! propose a change), pull requests opened from their changes, the repositories and defaults, and usage.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::assist::code::{self, check_repo, Forge, ForgeKind, PullRequest};
use crate::assist::runs::{CodeChange, Kind};
use crate::auth::{Admin, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::routes::assist_runs::{insert_run, within_limits, RunSummary, TimeCount};
use crate::routes::logs::TimeRange;
use crate::routes::portal_sources::PortalProvider;
use crate::state::AppState;

// ---------------------------------------------------------------- repositories

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewRepo {
    /// owner/name
    pub repo: String,
    /// github | gitlab
    pub provider: String,
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoView {
    pub repo: String,
    pub provider: String,
    pub services: Vec<String>,
    pub added_by: String,
    pub added_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeConnections {
    pub github: Option<String>,
    pub gitlab: Option<String>,
}

/// The org's GitHub and GitLab connections (account names), set in Developer Portal → Settings.
#[utoipa::path(get, path = "/api/v1/assist/code/connections", operation_id = "assist_code_connections", responses((status = 200, body = CodeConnections)), tag = "assist")]
pub async fn connections(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<CodeConnections> {
    let github = crate::routes::portal_home::stored_connector(&st, &ctx.org_id, PortalProvider::Github).await?.map(|c| c.account);
    let gitlab = crate::routes::portal_home::stored_connector(&st, &ctx.org_id, PortalProvider::Gitlab).await?.map(|c| c.account);
    Ok(Json(CodeConnections { github, gitlab }))
}

#[utoipa::path(get, path = "/api/v1/assist/code/repos", operation_id = "assist_list_code_repos", responses((status = 200, body = Vec<RepoView>)), tag = "assist")]
pub async fn list_repos(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<RepoView>> {
    let rows: Vec<(String, String, Vec<String>, String, i64)> =
        sqlx::query_as("SELECT repo, provider, services, added_by, (extract(epoch FROM added_at) * 1000)::bigint FROM assist_code_repos WHERE org_id = $1 ORDER BY repo").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(|(repo, provider, services, added_by, added_at)| RepoView { repo, provider, services, added_by, added_at }).collect()))
}

/// Adds a repository sessions may read and change, once the org's connection can reach it.
#[utoipa::path(post, path = "/api/v1/assist/code/repos", operation_id = "assist_add_code_repo", request_body = NewRepo, responses((status = 204)), tag = "assist")]
pub async fn add_repo(State(st): State<AppState>, Admin(ctx): Admin, Json(r): Json<NewRepo>) -> Result<StatusCode, ApiError> {
    let repo = r.repo.trim().trim_end_matches(".git").to_string();
    check_repo(&repo).map_err(ApiError::BadRequest)?;
    let (kind, provider) = match r.provider.as_str() {
        "github" => (ForgeKind::Github, PortalProvider::Github),
        "gitlab" => (ForgeKind::Gitlab, PortalProvider::Gitlab),
        _ => return Err(ApiError::BadRequest("provider is github or gitlab".into())),
    };
    let c = crate::routes::portal_home::stored_connector(&st, &ctx.org_id, provider).await?.ok_or_else(|| ApiError::BadRequest(format!("No {} connection: add one in Developer Portal → Settings", kind.as_str())))?;
    let forge = Forge { kind, base: c.base_url.trim_end_matches('/').to_string(), token: c.secret };
    code::base_branch(&st, &forge, &repo).await.map_err(|e| ApiError::BadRequest(format!("Can't reach {repo}: {e}")))?;
    let services: Vec<String> = r.services.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(50).collect();
    sqlx::query("INSERT INTO assist_code_repos (org_id, repo, provider, services, added_by) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (org_id, repo) DO UPDATE SET provider = $3, services = $4")
        .bind(&ctx.org_id)
        .bind(&repo)
        .bind(kind.as_str())
        .bind(&services)
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "assist.code.repo.add", &repo, kind.as_str()).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct RepoRef {
    pub repo: String,
}

#[utoipa::path(post, path = "/api/v1/assist/code/repos/remove", operation_id = "assist_remove_code_repo", request_body = RepoRef, responses((status = 204)), tag = "assist")]
pub async fn remove_repo(State(st): State<AppState>, Admin(ctx): Admin, Json(r): Json<RepoRef>) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM assist_code_repos WHERE org_id = $1 AND repo = $2").bind(&ctx.org_id).bind(&r.repo).execute(&st.postgres).await?;
    ctx.audit(&st, "assist.code.repo.remove", &r.repo, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- settings

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CodeSettings {
    /// Branches are named `<prefix><title>-<id>`.
    pub branch_prefix: String,
    /// Pull requests open as drafts.
    pub draft: bool,
    /// Open the pull request as soon as a session proposes a change.
    pub auto_pull_request: bool,
}

pub(crate) async fn settings(st: &AppState, org: &str) -> Result<CodeSettings, ApiError> {
    Ok(sqlx::query_as::<_, CodeSettings>("SELECT branch_prefix, draft, auto_pull_request FROM assist_code_settings WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?
        .unwrap_or(CodeSettings { branch_prefix: "assist/".into(), draft: true, auto_pull_request: false }))
}

#[utoipa::path(get, path = "/api/v1/assist/code/settings", operation_id = "assist_get_code_settings", responses((status = 200, body = CodeSettings)), tag = "assist")]
pub async fn get_settings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<CodeSettings> {
    Ok(Json(settings(&st, &ctx.org_id).await?))
}

#[utoipa::path(put, path = "/api/v1/assist/code/settings", operation_id = "assist_put_code_settings", request_body = CodeSettings, responses((status = 204)), tag = "assist")]
pub async fn put_settings(State(st): State<AppState>, Admin(ctx): Admin, Json(s): Json<CodeSettings>) -> Result<StatusCode, ApiError> {
    let prefix = s.branch_prefix.trim();
    if prefix.len() > 40 || !prefix.chars().all(|c| c.is_ascii_alphanumeric() || "-_/.".contains(c)) || prefix.starts_with('/') || prefix.contains("..") {
        return Err(ApiError::BadRequest("The branch prefix uses letters, digits, - _ . / (up to 40)".into()));
    }
    sqlx::query(
        "INSERT INTO assist_code_settings (org_id, branch_prefix, draft, auto_pull_request, updated_at) VALUES ($1, $2, $3, $4, now()) \
         ON CONFLICT (org_id) DO UPDATE SET branch_prefix = $2, draft = $3, auto_pull_request = $4, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(prefix)
    .bind(s.draft)
    .bind(s.auto_pull_request)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "assist.code.settings.update", "", &format!("prefix {prefix}, draft {}, automatic pull requests {}", s.draft, s.auto_pull_request)).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- sessions

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewSession {
    pub repo: String,
    /// What to fix or change.
    pub prompt: String,
}

async fn repo_added(st: &AppState, org: &str, repo: &str) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as("SELECT repo FROM assist_code_repos WHERE org_id = $1 AND repo = $2").bind(org).bind(repo).fetch_optional(&st.postgres).await?;
    found.map(|_| ()).ok_or_else(|| ApiError::BadRequest(format!("{repo} isn't added to Assist Code (Code → Settings → Repositories)")))
}

#[utoipa::path(post, path = "/api/v1/assist/code/sessions", operation_id = "assist_start_code_session", request_body = NewSession, responses((status = 200, body = RunSummary)), tag = "assist")]
pub async fn start_session(State(st): State<AppState>, Writer(ctx): Writer, Json(s): Json<NewSession>) -> ApiResult<RunSummary> {
    let prompt = s.prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > 8000 {
        return Err(ApiError::BadRequest("Describe the change in 1 to 8000 characters".into()));
    }
    repo_added(&st, &ctx.org_id, &s.repo).await?;
    within_limits(&st, &ctx.org_id).await?;
    let title: String = prompt.lines().next().unwrap_or(prompt).chars().take(100).collect();
    Ok(Json(insert_run(&st, &ctx.org_id, Kind::Code, &title, "manual", &json!({"repo": s.repo, "prompt": prompt}), &ctx.user_id, &ctx.name).await?))
}

/// Fills each proposed file's content on the default branch (none: a new file).
pub(crate) async fn with_before(st: &AppState, org: &str, repo: &str, mut c: CodeChange) -> CodeChange {
    if let Ok(f) = code::forge_for(st, org, repo).await {
        for file in &mut c.files {
            file.before = code::read_file(st, &f, repo, &file.path, None).await.ok();
        }
    }
    c
}

/// Opens the pull request of a run's change and records it on the run.
pub(crate) async fn open_for_run(st: &AppState, org: &str, run: &str, title: &str, repo: &str, change: &CodeChange) -> Result<PullRequest, ApiError> {
    let f = code::forge_for(st, org, repo).await.map_err(ApiError::BadRequest)?;
    let s = settings(st, org).await?;
    let branch = code::branch_name(&s.branch_prefix, &change.title, run);
    let files: Vec<code::FileChange> = change.files.iter().map(|x| code::FileChange { path: x.path.clone(), content: x.after.clone(), existed: x.before.is_some() }).collect();
    let link = format!("{}/assist/code/sessions/{run}", st.config.public_url);
    let body = format!("{}\n\n---\nProposed by an Assist Code session: {link} ({title})", change.description.trim());
    let pr = code::open_pull_request(st, &f, repo, &branch, &change.title, &body, &files, s.draft).await.map_err(ApiError::BadRequest)?;
    sqlx::query("UPDATE assist_runs SET pull_request = $3 WHERE org_id = $1 AND id = $2::uuid")
        .bind(org)
        .bind(run)
        .bind(sqlx::types::Json(json!({"url": pr.url, "number": pr.number, "branch": pr.branch, "draft": pr.draft, "state": "open"})))
        .execute(&st.postgres)
        .await?;
    Ok(pr)
}

/// Opens a pull (merge) request from the session's change, on a new branch.
#[utoipa::path(post, path = "/api/v1/assist/code/sessions/{id}/pull-request", operation_id = "assist_open_pull_request", params(("id" = String, Path)), responses((status = 200, body = PullRequest)), tag = "assist")]
pub async fn pull_request(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<PullRequest> {
    let run = crate::routes::assist_runs::get_run(State(st.clone()), ctx.clone(), Path(id.clone())).await?.0;
    if run.pull_request.is_some() {
        return Err(ApiError::Conflict("A pull request was already opened for this session".into()));
    }
    let change = run.change.ok_or_else(|| ApiError::BadRequest("This session proposed no change".into()))?;
    let repo = run.summary.subject["repo"].as_str().unwrap_or_default().to_string();
    let pr = open_for_run(&st, &ctx.org_id, &id, &run.summary.title, &repo, &change).await?;
    ctx.audit(&st, "assist.code.pull_request", &repo, &pr.url).await;
    Ok(Json(pr))
}

// ---------------------------------------------------------------- usage

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeUsageRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeUsage {
    pub sessions: i64,
    pub pull_requests_opened: i64,
    pub pull_requests_merged: i64,
    pub sessions_per_day: Vec<TimeCount>,
    pub pull_requests_per_day: Vec<TimeCount>,
}

/// Sessions and their pull requests in the range; open pull requests' states are refreshed first.
#[utoipa::path(post, path = "/api/v1/assist/code/usage", operation_id = "assist_code_usage", request_body = CodeUsageRequest, responses((status = 200, body = CodeUsage)), tag = "assist")]
pub async fn code_usage(State(st): State<AppState>, ctx: AuthContext, Json(r): Json<CodeUsageRequest>) -> ApiResult<CodeUsage> {
    let range = "org_id = $1 AND kind = 'code' AND created_at >= to_timestamp($2::float8 / 1000) AND created_at < to_timestamp($3::float8 / 1000)";
    let open: Vec<(String, Value, Value)> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT id::text, subject, pull_request FROM assist_runs WHERE {range} AND pull_request->>'state' = 'open' LIMIT 20")))
        .bind(&ctx.org_id)
        .bind(r.range.from)
        .bind(r.range.to)
        .fetch_all(&st.postgres)
        .await?;
    for (id, subject, pr) in open {
        let (Some(repo), Some(number)) = (subject["repo"].as_str(), pr["number"].as_i64()) else { continue };
        let Ok(f) = code::forge_for(&st, &ctx.org_id, repo).await else { continue };
        if let Ok(state) = code::pull_request_state(&st, &f, repo, number).await {
            sqlx::query("UPDATE assist_runs SET pull_request = jsonb_set(pull_request, '{state}', to_jsonb($3::text)) WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).bind(state).execute(&st.postgres).await?;
        }
    }
    let (sessions, opened, merged): (i64, i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT count(*), count(pull_request), count(*) FILTER (WHERE pull_request->>'state' = 'merged') FROM assist_runs WHERE {range}"
    )))
    .bind(&ctx.org_id)
    .bind(r.range.from)
    .bind(r.range.to)
    .fetch_one(&st.postgres)
    .await?;
    let per_day = |filter: &'static str| {
        let st = st.clone();
        let org = ctx.org_id.clone();
        async move {
            let rows: Vec<(i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT (extract(epoch FROM date_trunc('day', created_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') * 1000)::bigint, count(*) FROM assist_runs WHERE {range} {filter} GROUP BY 1 ORDER BY 1"
            )))
            .bind(&org)
            .bind(r.range.from)
            .bind(r.range.to)
            .fetch_all(&st.postgres)
            .await?;
            Ok::<_, ApiError>(rows.into_iter().map(|(at, count)| TimeCount { at, count }).collect::<Vec<_>>())
        }
    };
    Ok(Json(CodeUsage { sessions, pull_requests_opened: opened, pull_requests_merged: merged, sessions_per_day: per_day("").await?, pull_requests_per_day: per_day("AND pull_request IS NOT NULL").await? }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::llm::Provider;
    use crate::assist::testing::{fake_anthropic, sse_text, sse_tool};
    use crate::routes::assist_runs::{get_run, work_once};
    use crate::routes::logs::TimeRange;
    use serde_json::json;

    /// GitHub's REST API for one repository, scripted; the requests are kept.
    async fn fake_github() -> (String, std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
        use axum::extract::Request;
        use axum::response::IntoResponse;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
        let s = seen.clone();
        let app = axum::Router::new().fallback(move |req: Request| {
            let s = s.clone();
            async move {
                let what = format!("{} {}", req.method(), req.uri());
                let body = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap();
                s.lock().unwrap().push((what.clone(), serde_json::from_slice(&body).unwrap_or(Value::Null)));
                let old = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "def find_cart\n  Cart.all.find { |c| c.open? }\nend\n");
                let reply = match what.as_str() {
                    "GET /repos/example-org/storefront" => json!({"default_branch": "main"}),
                    w if w.starts_with("GET /repos/example-org/storefront/contents/app/cart.rb") => json!({"content": old, "encoding": "base64"}),
                    w if w.starts_with("GET /repos/example-org/storefront/git/ref/heads/main") => json!({"object": {"sha": "base1"}}),
                    "POST /repos/example-org/storefront/git/trees" => json!({"sha": "tree1"}),
                    "POST /repos/example-org/storefront/git/commits" => json!({"sha": "commit1"}),
                    "POST /repos/example-org/storefront/git/refs" => json!({"ref": "refs/heads/x"}),
                    "POST /repos/example-org/storefront/pulls" => json!({"html_url": "https://github.example.com/example-org/storefront/pull/7", "number": 7}),
                    "GET /repos/example-org/storefront/pulls/7" => json!({"state": "closed", "merged": true}),
                    _ => return (axum::http::StatusCode::NOT_FOUND, "{}").into_response(),
                };
                axum::Json(reply).into_response()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    /// A session reads the repository, proposes a change (shown with the files' old content), and a pull
    /// request is opened from it as a draft on a new branch; usage counts it, merged once GitHub says so.
    /// Skipped unless OC_TEST_POSTGRES_URL and OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn a_session_proposes_a_change_and_opens_a_pull_request() {
        let new = "def find_cart\n  Cart.where(open: true).first\nend\n";
        let (base_url, _) = fake_anthropic(vec![
            sse_tool("t1", "read_repo_file", json!({"repo": "example-org/storefront", "path": "app/cart.rb"})),
            sse_tool("t2", "propose_change", json!({"title": "Query open carts instead of scanning", "description": "Cart.all loads every cart", "files": [{"path": "app/cart.rb", "content": new}]})),
            sse_tool("t3", "conclude", json!({"summary": "Cart lookups scanned all carts", "verdict": "change_proposed"})),
            sse_text("Done."),
        ])
        .await;
        let (github, gh_seen) = fake_github().await;
        let Some((_, states)) = crate::test_support::replicas_with(1, |cfg| {
            cfg.assist = Ok(Some(Provider::Anthropic { base_url: base_url.clone(), api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }));
            cfg.portal_allow_private = true;
        })
        .await
        else {
            return;
        };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "code").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let ctx = AuthContext { user_id, ..admin };
        sqlx::query("INSERT INTO portal_connectors (org_id, provider, base_url, account, secret_enc) VALUES ($1, 'github', $2, 'assist-bot', $3)")
            .bind(&ctx.org_id)
            .bind(&github)
            .bind(st.secrets.encrypt("ghp_test"))
            .execute(&st.postgres)
            .await
            .unwrap();

        assert!(matches!(start_session(State(st.clone()), Writer(ctx.clone()), Json(NewSession { repo: "example-org/storefront".into(), prompt: "x".into() })).await, Err(ApiError::BadRequest(_))), "not added yet");
        add_repo(State(st.clone()), Admin(ctx.clone()), Json(NewRepo { repo: "example-org/storefront".into(), provider: "github".into(), services: vec!["storefront-api".into()] })).await.unwrap();
        assert_eq!(list_repos(State(st.clone()), ctx.clone()).await.unwrap().0[0].services, vec!["storefront-api"]);

        let run = start_session(State(st.clone()), Writer(ctx.clone()), Json(NewSession { repo: "example-org/storefront".into(), prompt: "Cart lookups are slow in checkout".into() })).await.unwrap().0;
        assert_eq!(work_once(&st, &ctx.org_id).await, 1);
        let r = get_run(State(st.clone()), ctx.clone(), Path(run.id.clone())).await.unwrap().0;
        assert_eq!((r.summary.status.as_str(), r.summary.verdict.as_deref()), ("concluded", Some("change_proposed")), "{:?}", r.summary.error);
        let change = r.change.as_ref().unwrap();
        assert!(change.files[0].before.as_deref().unwrap().contains("Cart.all") && change.files[0].after.contains("where(open: true)"));

        let pr = pull_request(State(st.clone()), Writer(ctx.clone()), Path(run.id.clone())).await.unwrap().0;
        assert_eq!((pr.number, pr.draft), (7, true));
        assert!(pr.branch.starts_with("assist/query-open-carts-instead-of-scanning-"), "{}", pr.branch);
        let calls = gh_seen.lock().unwrap().clone();
        let tree = calls.iter().find(|c| c.0 == "POST /repos/example-org/storefront/git/trees").unwrap();
        assert_eq!((tree.1["base_tree"].as_str(), tree.1["tree"][0]["content"].as_str()), (Some("base1"), Some(new)));
        let pull = calls.iter().find(|c| c.0 == "POST /repos/example-org/storefront/pulls").unwrap();
        assert_eq!((pull.1["base"].as_str(), pull.1["draft"].as_bool()), (Some("main"), Some(true)));
        assert!(matches!(pull_request(State(st.clone()), Writer(ctx.clone()), Path(run.id.clone())).await, Err(ApiError::Conflict(_))), "only once");

        let now = chrono::Utc::now().timestamp_millis();
        let u = code_usage(State(st.clone()), ctx.clone(), Json(CodeUsageRequest { range: TimeRange { from: now - 3_600_000, to: now + 60_000 } })).await.unwrap().0;
        assert_eq!((u.sessions, u.pull_requests_opened, u.pull_requests_merged), (1, 1, 1));
    }
}
