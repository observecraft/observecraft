//! Assist Agent Builder (Automation → Agent Builder): custom agents with their own instructions and
//! skills, run by hand or on a schedule as their owner. Their runs are Assist runs of kind `agent`
//! (`assist_runs.rs`); a description can be turned into a draft agent by the model.

use std::sync::Mutex;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::assist::agent::{Loop, Tools};
use crate::assist::llm::{Msg, ToolSpec, Usage};
use crate::assist::runs::Kind;
use crate::assist::tools::SKILLS;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::routes::assist_runs::{insert_run, within_limits, RunSummary};
use crate::routes::workflow_cron::{zone, Cron};
use crate::state::AppState;

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    /// Skills (groups of Assist's tools) the agent may use.
    pub skills: Vec<String>,
    /// May use tools that change something without asking.
    #[serde(default)]
    pub allow_actions: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Five-field cron (`0 9 * * 1-5`); none: by hand only.
    pub schedule: Option<String>,
    /// IANA zone of the schedule (default UTC).
    pub timezone: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// agent (default) | code: an Assist Code automation on `repo`.
    #[serde(default)]
    pub kind: Option<String>,
    /// The repository of a code automation.
    #[serde(default)]
    pub repo: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: String,
    /// agent | code
    pub kind: String,
    pub repo: Option<String>,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub skills: Vec<String>,
    pub allow_actions: bool,
    pub tags: Vec<String>,
    pub schedule: Option<String>,
    pub timezone: String,
    pub enabled: bool,
    pub next_run_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub runs: i64,
    pub last_run_at: Option<i64>,
    pub last_run_status: Option<String>,
    pub last_verdict: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    kind: String,
    repo: Option<String>,
    name: String,
    description: String,
    instructions: String,
    skills: Vec<String>,
    allow_actions: bool,
    tags: Vec<String>,
    schedule: Option<String>,
    timezone: String,
    enabled: bool,
    next_run_at: Option<i64>,
    created_by: String,
    created_at: i64,
    updated_at: i64,
    runs: i64,
    last_run_at: Option<i64>,
    last_run_status: Option<String>,
    last_verdict: Option<String>,
}

const SELECT: &str = "SELECT a.id::text AS id, a.kind, a.repo, a.name, a.description, a.instructions, a.skills, a.allow_actions, a.tags, a.schedule, a.timezone, a.enabled, \
    (extract(epoch FROM a.next_run_at) * 1000)::bigint AS next_run_at, a.created_by, (extract(epoch FROM a.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM a.updated_at) * 1000)::bigint AS updated_at, \
    (SELECT count(*) FROM assist_runs r WHERE r.org_id = a.org_id AND r.kind = 'agent' AND r.subject->>'agentId' = a.id::text) AS runs, \
    l.at AS last_run_at, l.status AS last_run_status, l.verdict AS last_verdict FROM assist_agents a LEFT JOIN LATERAL (SELECT (extract(epoch FROM r.created_at) * 1000)::bigint AS at, r.status, r.verdict FROM assist_runs r \
    WHERE r.org_id = a.org_id AND r.kind = 'agent' AND r.subject->>'agentId' = a.id::text ORDER BY r.created_at DESC LIMIT 1) l ON true";

fn view(r: AgentRow) -> AgentView {
    let AgentRow { id, kind, repo, name, description, instructions, skills, allow_actions, tags, schedule, timezone, enabled, next_run_at, created_by, created_at, updated_at, runs, last_run_at, last_run_status, last_verdict } = r;
    AgentView { id, kind, repo, name, description, instructions, skills, allow_actions, tags, schedule, timezone, enabled, next_run_at, created_by, created_at, updated_at, runs, last_run_at, last_run_status, last_verdict }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<AgentView, ApiError> {
    if !is_uuid(id) {
        return Err(ApiError::NotFound("No such agent".into()));
    }
    let row: Option<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE a.org_id = $1 AND a.id = $2::uuid"))).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(view).ok_or_else(|| ApiError::NotFound("No such agent".into()))
}

/// What a run of the agent needs.
pub(crate) struct ForRun {
    pub instructions: String,
    pub skills: Vec<String>,
    pub allow_actions: bool,
}

pub(crate) async fn for_run(st: &AppState, org: &str, id: &str) -> Result<Option<ForRun>, ApiError> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<(String, Vec<String>, bool)> = sqlx::query_as("SELECT instructions, skills, allow_actions FROM assist_agents WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).fetch_optional(&st.postgres).await?;
    Ok(row.map(|(instructions, skills, allow_actions)| ForRun { instructions, skills, allow_actions }))
}

/// The next run after `now`, or why the schedule is wrong.
fn next_run(schedule: &str, timezone: &str, now: i64) -> Result<i64, String> {
    let tz = zone(timezone)?;
    Cron::parse(schedule)?.next_after(now, tz).ok_or_else(|| "The schedule never runs".into())
}

/// The input checked, with the next run when scheduled.
fn check(a: &AgentInput) -> Result<(String, Option<String>, Option<i64>), ApiError> {
    let bad = |m: String| ApiError::BadRequest(m);
    match (a.kind.as_deref().unwrap_or("agent"), a.repo.as_deref()) {
        ("agent", _) => {}
        ("code", Some(repo)) => crate::assist::code::check_repo(repo).map_err(bad)?,
        ("code", None) => return Err(bad("A code automation needs a repository".into())),
        (other, _) => return Err(bad(format!("Unknown kind {other}"))),
    }
    if a.name.trim().is_empty() || a.name.chars().count() > 100 {
        return Err(bad("The name is 1 to 100 characters".into()));
    }
    if a.instructions.trim().is_empty() || a.instructions.chars().count() > 8000 {
        return Err(bad("The instructions are 1 to 8000 characters".into()));
    }
    if let Some(s) = a.skills.iter().find(|s| !SKILLS.iter().any(|k| k.name == s.as_str())) {
        return Err(bad(format!("Unknown skill {s}")));
    }
    let timezone = a.timezone.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "UTC".into());
    let schedule = a.schedule.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let next = match &schedule {
        Some(s) => Some(next_run(s, &timezone, chrono::Utc::now().timestamp_millis()).map_err(|e| bad(format!("Schedule: {e}")))?),
        None => None,
    };
    Ok((timezone, schedule, next))
}

#[utoipa::path(get, path = "/api/v1/assist/agents", operation_id = "assist_list_agents", responses((status = 200, body = Vec<AgentView>)), tag = "assist")]
pub async fn list_agents(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AgentView>> {
    let rows: Vec<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE a.org_id = $1 ORDER BY a.updated_at DESC"))).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(view).collect()))
}

#[utoipa::path(get, path = "/api/v1/assist/agents/{id}", operation_id = "assist_get_agent", params(("id" = String, Path)), responses((status = 200, body = AgentView)), tag = "assist")]
pub async fn get_agent(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<AgentView> {
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/assist/agents", operation_id = "assist_create_agent", request_body = AgentInput, responses((status = 200, body = AgentView)), tag = "assist")]
pub async fn create_agent(State(st): State<AppState>, Writer(ctx): Writer, Json(a): Json<AgentInput>) -> ApiResult<AgentView> {
    let (timezone, schedule, next) = check(&a)?;
    let id: String = sqlx::query_scalar(
        "INSERT INTO assist_agents (org_id, name, description, instructions, skills, allow_actions, tags, schedule, timezone, enabled, next_run_at, owner, created_by, kind, repo) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, to_timestamp($11::float8 / 1000), $12::uuid, $13, $14, $15) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(a.name.trim())
    .bind(a.description.trim())
    .bind(a.instructions.trim())
    .bind(&a.skills)
    .bind(a.allow_actions)
    .bind(&a.tags)
    .bind(&schedule)
    .bind(&timezone)
    .bind(a.enabled)
    .bind(next)
    .bind(&ctx.user_id)
    .bind(&ctx.name)
    .bind(a.kind.as_deref().unwrap_or("agent"))
    .bind(&a.repo)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "assist.agent.create", &id, a.name.trim()).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// Saves the agent; it then runs as the member who saved it.
#[utoipa::path(put, path = "/api/v1/assist/agents/{id}", operation_id = "assist_update_agent", params(("id" = String, Path)), request_body = AgentInput, responses((status = 200, body = AgentView)), tag = "assist")]
pub async fn update_agent(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(a): Json<AgentInput>) -> ApiResult<AgentView> {
    load(&st, &ctx.org_id, &id).await?;
    let (timezone, schedule, next) = check(&a)?;
    sqlx::query(
        "UPDATE assist_agents SET name = $3, description = $4, instructions = $5, skills = $6, allow_actions = $7, tags = $8, schedule = $9, timezone = $10, enabled = $11, \
         next_run_at = to_timestamp($12::float8 / 1000), owner = $13::uuid, repo = $14, updated_at = now() WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(a.name.trim())
    .bind(a.description.trim())
    .bind(a.instructions.trim())
    .bind(&a.skills)
    .bind(a.allow_actions)
    .bind(&a.tags)
    .bind(&schedule)
    .bind(&timezone)
    .bind(a.enabled)
    .bind(next)
    .bind(&ctx.user_id)
    .bind(&a.repo)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "assist.agent.update", &id, a.name.trim()).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/assist/agents/{id}", operation_id = "assist_delete_agent", params(("id" = String, Path)), responses((status = 204)), tag = "assist")]
pub async fn delete_agent(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let a = load(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM assist_agents WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "assist.agent.delete", &id, &a.name).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunAgent {
    /// Extra input for this run.
    pub input: Option<String>,
}

#[utoipa::path(post, path = "/api/v1/assist/agents/{id}/run", operation_id = "assist_run_agent", params(("id" = String, Path)), request_body = RunAgent, responses((status = 200, body = RunSummary)), tag = "assist")]
pub async fn run_agent(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(r): Json<RunAgent>) -> ApiResult<RunSummary> {
    let a = load(&st, &ctx.org_id, &id).await?;
    let input = r.input.unwrap_or_default();
    if input.chars().count() > 4000 {
        return Err(ApiError::BadRequest("The input is up to 4000 characters".into()));
    }
    within_limits(&st, &ctx.org_id).await?;
    let (kind, subject) = run_of(&a.kind, &id, a.repo.as_deref(), &a.instructions, &input);
    Ok(Json(insert_run(&st, &ctx.org_id, kind, &a.name, "manual", &subject, &ctx.user_id, &ctx.name).await?))
}

/// A run of an agent, or a coding session of a code automation.
fn run_of(kind: &str, id: &str, repo: Option<&str>, instructions: &str, input: &str) -> (Kind, Value) {
    match (kind, repo) {
        ("code", Some(repo)) => (Kind::Code, json!({"agentId": id, "repo": repo, "prompt": instructions, "input": input})),
        _ => (Kind::Agent, json!({"agentId": id, "input": input})),
    }
}

/// Queues a run of every scheduled agent that is due (any server; rows are claimed) and moves each to
/// its next time. A run over the org's hourly limit is skipped, not delayed.
pub async fn enqueue_due(st: &AppState) -> Result<usize, ApiError> {
    let mut tx = st.postgres.begin().await?;
    type Due = (String, String, String, String, String, String, String, Option<String>, String);
    let due: Vec<Due> = sqlx::query_as(
        "SELECT org_id, id::text, name, schedule, timezone, owner::text, kind, repo, instructions FROM assist_agents \
         WHERE enabled AND schedule IS NOT NULL AND owner IS NOT NULL AND next_run_at <= now() ORDER BY next_run_at LIMIT 50 FOR UPDATE SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut queued = Vec::new();
    for (org, id, name, schedule, timezone, owner, kind, repo, instructions) in &due {
        let next = next_run(schedule, timezone, now).ok();
        sqlx::query("UPDATE assist_agents SET next_run_at = to_timestamp($3::float8 / 1000) WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).bind(next).execute(&mut *tx).await?;
        let (kind, subject) = run_of(kind, id, repo.as_deref(), instructions, "");
        queued.push((org.clone(), name.clone(), owner.clone(), kind, subject));
    }
    tx.commit().await?;
    let mut n = 0;
    for (org, name, owner, kind, subject) in queued {
        if within_limits(st, &org).await.is_ok() {
            insert_run(st, &org, kind, &name, "schedule", &subject, &owner, "Schedule").await?;
            n += 1;
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------- drafts

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftRequest {
    /// What the agent should monitor, investigate or automate.
    pub description: String,
}

#[derive(Serialize, Deserialize, ToSchema, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub skills: Vec<String>,
    pub schedule: Option<String>,
}

/// The one tool of a draft: the agent, as the model defines it.
struct DraftTools(Mutex<Option<AgentDraft>>);

impl Tools for DraftTools {
    fn specs(&self) -> Vec<ToolSpec> {
        let skills: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        vec![ToolSpec {
            name: "define_agent".into(),
            description: "Define the agent. Call it once.".into(),
            schema: json!({"type": "object", "properties": {
                "name": {"type": "string", "description": "Short, e.g. Daily Error Triager"},
                "description": {"type": "string", "description": "One sentence"},
                "instructions": {"type": "string", "description": "What the agent does on each run, step by step, and what it reports"},
                "skills": {"type": "array", "items": {"type": "string", "enum": skills}},
                "schedule": {"type": "string", "description": "Five-field cron when it should run regularly (0 9 * * 1-5); omit to run by hand"},
            }, "required": ["name", "instructions", "skills"]}),
        }]
    }

    fn writes(&self, _: &str) -> bool {
        false
    }

    async fn run(&self, _: &str, input: &Value) -> Result<String, String> {
        let mut d: AgentDraft = serde_json::from_value(json!({
            "name": input["name"], "description": input["description"].as_str().unwrap_or(""), "instructions": input["instructions"],
            "skills": input.get("skills").cloned().unwrap_or(json!([])), "schedule": input.get("schedule"),
        }))
        .map_err(|e| format!("Invalid agent: {e}"))?;
        d.skills.retain(|s| SKILLS.iter().any(|k| k.name == s.as_str()));
        if d.schedule.as_deref().is_some_and(|s| Cron::parse(s).is_err()) {
            d.schedule = None;
        }
        *self.0.lock().unwrap() = Some(d);
        Ok("Defined.".into())
    }
}

/// Turns a description into a draft agent (not saved).
#[utoipa::path(post, path = "/api/v1/assist/agents/draft", operation_id = "assist_draft_agent", request_body = DraftRequest, responses((status = 200, body = AgentDraft)), tag = "assist")]
pub async fn draft_agent(State(st): State<AppState>, Writer(ctx): Writer, Json(r): Json<DraftRequest>) -> ApiResult<AgentDraft> {
    let description = r.description.trim();
    if description.is_empty() || description.chars().count() > 4000 {
        return Err(ApiError::BadRequest("Describe the agent in 1 to 4000 characters".into()));
    }
    let resolved = crate::routes::assist_provider::resolve(&st, &ctx.org_id).await?.ok_or_else(|| ApiError::BadRequest("Assist has no model: an admin sets one in Organization Settings → Assist AI".into()))?;
    let model = crate::assist::http::HttpModel { provider: resolved.provider, max_tokens: 8_000, reach: resolved.reach };
    let skills = SKILLS.iter().map(|s| format!("- {}: {}", s.name, s.description)).collect::<Vec<_>>().join("\n");
    let system = format!(
        "You design custom agents for Assist, the AI assistant of ObserveCraft (an observability platform). From the user's description, define one \
agent with define_agent: a short name, a one-sentence description, clear step-by-step instructions for each run (what to look at, how to decide, \
what to report) and only the skills it needs. Give a cron schedule only if the description asks for regular runs.\n\nSkills:\n{skills}\n"
    );
    let tools = DraftTools(Mutex::new(None));
    let l = Loop { model: &model, tools: &tools, system: &system, auto_approve: false };
    let mut msgs = vec![Msg::User(description.to_string())];
    let outcome = l.run(&mut msgs, &mut Usage::default(), &mut |_| {}).await;
    match (tools.0.into_inner().unwrap(), outcome) {
        (Some(d), _) => Ok(Json(d)),
        (None, crate::assist::agent::Outcome::Failed(e)) => Err(ApiError::BadRequest(e)),
        (None, _) => Err(ApiError::BadRequest("The model didn't define an agent; describe it in more detail".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::llm::Provider;
    use crate::assist::testing::{fake_anthropic, sse_text, sse_tool};
    use crate::routes::assist_runs::{get_run, work_once};
    use serde_json::json;

    fn input(name: &str, schedule: Option<&str>) -> AgentInput {
        AgentInput {
            name: name.into(),
            description: "Finds new error issues every morning".into(),
            instructions: "List error issues first seen in the last day and say which need attention.".into(),
            skills: vec!["errors".into(), "logs".into()],
            allow_actions: false,
            tags: vec!["team:orders".into()],
            schedule: schedule.map(Into::into),
            timezone: Some("Europe/Bucharest".into()),
            enabled: true,
            kind: None,
            repo: None,
        }
    }

    /// An agent is saved, run by hand with only its skills and its instructions, run again when its
    /// schedule is due, and drafted from a description. Skipped unless OC_TEST_POSTGRES_URL and
    /// OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn an_agent_runs_by_hand_and_on_its_schedule() {
        let draft = json!({"name": "Daily Error Triager", "description": "Triage new errors", "instructions": "Every morning, list new error issues.", "skills": ["errors", "logs", "nonsense"], "schedule": "0 9 * * *"});
        let done = json!({"summary": "2 new issues; 1 needs attention", "verdict": "needs_attention"});
        let (base_url, seen) = fake_anthropic(vec![sse_tool("d1", "define_agent", draft), sse_text("Drafted."), sse_tool("t1", "conclude", done.clone()), sse_text("Done."), sse_tool("t2", "conclude", done), sse_text("Done.")]).await;
        let Some((_, states)) = crate::test_support::replicas_with(1, |cfg| {
            cfg.assist = Ok(Some(Provider::Anthropic { base_url: base_url.clone(), api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }));
        })
        .await
        else {
            return;
        };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "agents").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let ctx = AuthContext { user_id, ..admin };

        // Drafted from a description: unknown skills are dropped.
        let d = draft_agent(State(st.clone()), Writer(ctx.clone()), Json(DraftRequest { description: "Every morning find new errors and tell me which matter".into() })).await.unwrap().0;
        assert_eq!((d.name.as_str(), d.skills.clone(), d.schedule.as_deref()), ("Daily Error Triager", vec!["errors".to_string(), "logs".to_string()], Some("0 9 * * *")));

        assert!(matches!(create_agent(State(st.clone()), Writer(ctx.clone()), Json(input("Bad", Some("every morning")))).await, Err(ApiError::BadRequest(_))));
        let a = create_agent(State(st.clone()), Writer(ctx.clone()), Json(input("Daily Error Triager", Some("0 9 * * *")))).await.unwrap().0;
        assert!(a.next_run_at.is_some());

        // By hand.
        let run = run_agent(State(st.clone()), Writer(ctx.clone()), Path(a.id.clone()), Json(RunAgent { input: Some("Only billing".into()) })).await.unwrap().0;
        assert_eq!(work_once(&st, &ctx.org_id).await, 1);
        let r = get_run(State(st.clone()), ctx.clone(), Path(run.id.clone())).await.unwrap().0;
        assert_eq!((r.summary.status.as_str(), r.summary.verdict.as_deref()), ("concluded", Some("needs_attention")), "{:?}", r.summary.error);
        let req = seen.lock().unwrap()[2].clone();
        assert!(req["system"].as_str().unwrap().contains("List error issues first seen in the last day"));
        let tools: Vec<String> = req["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(tools, vec!["search_logs", "search_error_issues", "conclude"]);
        assert!(req["messages"][0]["content"].to_string().contains("Only billing"));

        // On schedule, once due.
        assert_eq!(enqueue_due(&st).await.unwrap(), 0);
        sqlx::query("UPDATE assist_agents SET next_run_at = now() - interval '1 minute' WHERE org_id = $1").bind(&ctx.org_id).execute(&st.postgres).await.unwrap();
        assert_eq!(enqueue_due(&st).await.unwrap(), 1);
        assert_eq!(work_once(&st, &ctx.org_id).await, 1);
        let listed = list_agents(State(st.clone()), ctx.clone()).await.unwrap().0;
        let l = listed.iter().find(|x| x.id == a.id).unwrap();
        assert_eq!((l.runs, l.last_run_status.as_deref()), (2, Some("concluded")));
        assert!(l.next_run_at.unwrap() > chrono::Utc::now().timestamp_millis());

        delete_agent(State(st.clone()), Writer(ctx.clone()), Path(a.id.clone())).await.unwrap();
        assert!(list_agents(State(st.clone()), ctx.clone()).await.unwrap().0.iter().all(|x| x.id != a.id));
    }
}
