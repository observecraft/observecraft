//! Assist (AI chat): conversations with the install's model, which reads the org's data through
//! tools (`assist::tools`) as the signed-in user and asks before changing anything. A reply streams as
//! Server-Sent Events; a reply waiting for approval is resumed by `…/approve`. Conversations, settings
//! and usage are Postgres `assist_*` (migration 0075); each user sees only their own conversations.
use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::assist::agent::{self, Loop, Outcome};
use crate::assist::http::HttpModel;
use crate::assist::llm::{Msg, Provider, ToolCall, Usage};
use crate::assist::prompt::{system_prompt, PromptInput};
use crate::assist::tools::{self, OrgTools};
use crate::auth::AuthContext;
use crate::routes::assist_provider::resolve;
use crate::error::{ApiError, ApiResult};
use crate::routes::logs::TimeRange;
use crate::state::AppState;

// ---------------------------------------------------------------- status and settings

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistSkill {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    /// Some of its tools change something (they ask for approval).
    pub writes: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistStatus {
    /// A model is set up: the organization's own or the install's (`OC_ASSIST_PROVIDER`).
    pub configured: bool,
    /// `org` or `install`.
    pub source: Option<String>,
    /// Why the organization's provider can't be used.
    pub error: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub skills: Vec<AssistSkill>,
}

#[utoipa::path(get, path = "/api/v1/assist/status", operation_id = "assist_status", responses((status = 200, body = AssistStatus)), tag = "assist")]
pub async fn status(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<AssistStatus> {
    let (resolved, error) = match resolve(&st, &ctx.org_id).await {
        Ok(r) => (r, None),
        Err(ApiError::BadRequest(e)) => (None, Some(e)),
        Err(e) => return Err(e),
    };
    let p = resolved.as_ref().map(|r| &r.provider);
    let catalog = tools::catalog();
    Ok(Json(AssistStatus {
        configured: p.is_some(),
        source: resolved.as_ref().map(|r| r.source.to_string()),
        error,
        provider: p.map(|p| p.kind().to_string()),
        model: p.map(|p| p.model().to_string()),
        skills: tools::SKILLS
            .iter()
            .map(|s| AssistSkill {
                name: s.name.into(),
                description: s.description.into(),
                tools: catalog.iter().filter(|c| c.1 == s.name).map(|c| c.0.to_string()).collect(),
                writes: catalog.iter().any(|c| c.1 == s.name && c.2),
            })
            .collect(),
    }))
}

#[derive(Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssistSettings {
    /// Interests, preferences or instructions the model keeps in mind.
    pub context: String,
    pub disabled_skills: Vec<String>,
}

const MAX_CONTEXT: usize = 4_000;

async fn load_settings(st: &AppState, ctx: &AuthContext) -> Result<AssistSettings, ApiError> {
    let row: Option<(String, Vec<String>)> = sqlx::query_as("SELECT context, disabled_skills FROM assist_settings WHERE org_id = $1 AND user_id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(|(context, disabled_skills)| AssistSettings { context, disabled_skills }).unwrap_or_default())
}

#[utoipa::path(get, path = "/api/v1/assist/settings", operation_id = "assist_get_settings", responses((status = 200, body = AssistSettings)), tag = "assist")]
pub async fn get_settings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<AssistSettings> {
    Ok(Json(load_settings(&st, &ctx).await?))
}

#[utoipa::path(put, path = "/api/v1/assist/settings", operation_id = "assist_update_settings", request_body = AssistSettings, responses((status = 204)), tag = "assist")]
pub async fn update_settings(State(st): State<AppState>, ctx: AuthContext, Json(s): Json<AssistSettings>) -> Result<StatusCode, ApiError> {
    if s.context.chars().count() > MAX_CONTEXT {
        return Err(ApiError::BadRequest(format!("The user context is limited to {MAX_CONTEXT} characters")));
    }
    if let Some(unknown) = s.disabled_skills.iter().find(|d| !tools::SKILLS.iter().any(|k| k.name == d.as_str())) {
        return Err(ApiError::BadRequest(format!("Unknown skill {unknown}")));
    }
    sqlx::query(
        "INSERT INTO assist_settings (org_id, user_id, context, disabled_skills) VALUES ($1, $2::uuid, $3, $4) \
         ON CONFLICT (org_id, user_id) DO UPDATE SET context = EXCLUDED.context, disabled_skills = EXCLUDED.disabled_skills, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .bind(s.context.trim())
    .bind(&s.disabled_skills)
    .execute(&st.postgres)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- conversations

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub surface: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewTool {
    pub id: String,
    pub name: String,
    #[schema(value_type = Object)]
    pub input: Value,
    /// ok | error | pending (waiting for approval) | running
    pub status: String,
    pub summary: String,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewMessage {
    /// user | assistant
    pub role: String,
    pub text: String,
    pub tools: Vec<ViewTool>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationView {
    pub id: String,
    pub title: String,
    pub surface: String,
    pub messages: Vec<ViewMessage>,
    /// Tool calls wait for the user's approval.
    pub awaiting_approval: bool,
}

/// What the user sees of the model's messages: their messages, and the model's turns of each reply
/// merged into one answer with the tools it used.
pub fn view(msgs: &[Msg], pending: &[ToolCall]) -> Vec<ViewMessage> {
    let mut out: Vec<ViewMessage> = Vec::new();
    for m in msgs {
        match m {
            Msg::User(text) => out.push(ViewMessage { role: "user".into(), text: text.clone(), tools: Vec::new() }),
            Msg::Assistant { text, calls, .. } => {
                if out.last().is_none_or(|l| l.role != "assistant") {
                    out.push(ViewMessage { role: "assistant".into(), text: String::new(), tools: Vec::new() });
                }
                let last = out.last_mut().expect("pushed");
                if !text.trim().is_empty() {
                    if !last.text.is_empty() {
                        last.text.push_str("\n\n");
                    }
                    last.text.push_str(text.trim());
                }
                last.tools.extend(calls.iter().map(|c| ViewTool {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    input: c.input.clone().unwrap_or(Value::Null),
                    status: if pending.iter().any(|p| p.id == c.id) { "pending" } else { "running" }.into(),
                    summary: String::new(),
                }));
            }
            Msg::Results(results) => {
                if let Some(last) = out.last_mut() {
                    for r in results {
                        if let Some(t) = last.tools.iter_mut().find(|t| t.id == r.id) {
                            t.status = if r.is_error { "error" } else { "ok" }.into();
                            t.summary = agent::truncate(r.content.lines().next().unwrap_or(""), 160);
                        }
                    }
                }
            }
        }
    }
    out
}

#[derive(sqlx::FromRow)]
struct ConversationRow {
    title: String,
    surface: String,
    messages: sqlx::types::Json<Vec<Msg>>,
    pending: Option<sqlx::types::Json<Vec<ToolCall>>>,
}

fn is_uuid(id: &str) -> bool {
    id.len() == 36 && id.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

async fn load(st: &AppState, ctx: &AuthContext, id: &str) -> Result<ConversationRow, ApiError> {
    if !is_uuid(id) {
        return Err(ApiError::NotFound("Conversation not found".into()));
    }
    sqlx::query_as::<_, ConversationRow>("SELECT title, surface, messages, pending FROM assist_conversations WHERE org_id = $1 AND user_id = $2::uuid AND id = $3::uuid")
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Conversation not found".into()))
}

#[utoipa::path(get, path = "/api/v1/assist/conversations", operation_id = "assist_list_conversations", responses((status = 200, body = Vec<ConversationSummary>)), tag = "assist")]
pub async fn list_conversations(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<ConversationSummary>> {
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id::text, title, surface, (extract(epoch FROM updated_at) * 1000)::bigint FROM assist_conversations \
         WHERE org_id = $1 AND user_id = $2::uuid ORDER BY updated_at DESC LIMIT 100",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows.into_iter().map(|(id, title, surface, updated_at)| ConversationSummary { id, title, surface, updated_at }).collect()))
}

#[utoipa::path(get, path = "/api/v1/assist/conversations/{id}", operation_id = "assist_get_conversation", params(("id" = String, Path)), responses((status = 200, body = ConversationView)), tag = "assist")]
pub async fn get_conversation(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<ConversationView> {
    let row = load(&st, &ctx, &id).await?;
    let pending = row.pending.map(|p| p.0).unwrap_or_default();
    Ok(Json(ConversationView { id, title: row.title, surface: row.surface, messages: view(&row.messages.0, &pending), awaiting_approval: !pending.is_empty() }))
}

#[utoipa::path(delete, path = "/api/v1/assist/conversations/{id}", operation_id = "assist_delete_conversation", params(("id" = String, Path)), responses((status = 204)), tag = "assist")]
pub async fn delete_conversation(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &ctx, &id).await?;
    sqlx::query("DELETE FROM assist_conversations WHERE org_id = $1 AND user_id = $2::uuid AND id = $3::uuid").bind(&ctx.org_id).bind(&ctx.user_id).bind(&id).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- replies

/// One Server-Sent Event of a reply; the event name is its `type`.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistEvent {
    /// First, with the conversation the reply belongs to.
    Conversation { id: String, title: String },
    Text { text: String },
    ToolStart {
        id: String,
        name: String,
        #[schema(value_type = Object)]
        input: Value,
    },
    ToolEnd { id: String, ok: bool, summary: String },
    Approval {
        id: String,
        name: String,
        #[schema(value_type = Object)]
        input: Value,
    },
    Error { message: String },
    /// Last. `awaiting_approval`: the reply continues after `…/approve`.
    Done { awaiting_approval: bool, input_tokens: u64, output_tokens: u64 },
}

impl From<agent::Event> for AssistEvent {
    fn from(e: agent::Event) -> Self {
        match e {
            agent::Event::Text { text } => AssistEvent::Text { text },
            agent::Event::ToolStart { id, name, input } => AssistEvent::ToolStart { id, name, input },
            agent::Event::ToolEnd { id, ok, summary } => AssistEvent::ToolEnd { id, ok, summary },
            agent::Event::Approval { id, name, input } => AssistEvent::Approval { id, name, input },
        }
    }
}

impl AssistEvent {
    fn name(&self) -> &'static str {
        match self {
            AssistEvent::Conversation { .. } => "conversation",
            AssistEvent::Text { .. } => "text",
            AssistEvent::ToolStart { .. } => "tool_start",
            AssistEvent::ToolEnd { .. } => "tool_end",
            AssistEvent::Approval { .. } => "approval",
            AssistEvent::Error { .. } => "error",
            AssistEvent::Done { .. } => "done",
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    /// Continue this conversation; a new one when absent.
    pub conversation_id: Option<String>,
    pub message: String,
    /// chat (Assist Chat) | panel (Ask Assist on another page); for reports.
    #[serde(default)]
    pub surface: Option<String>,
    /// Run tools that change something without asking.
    #[serde(default)]
    pub auto_approve: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApproveRequest {
    /// The ids of the calls the user approved; the other pending ones are declined.
    pub approved: Vec<String>,
    #[serde(default)]
    pub auto_approve: bool,
}


const MAX_MESSAGE: usize = 20_000;

fn title_of(message: &str) -> String {
    let line = message.lines().find(|l| !l.trim().is_empty()).unwrap_or("New conversation").trim();
    agent::truncate(line, 80).lines().next().unwrap_or("").to_string()
}

/// What a reply needs, checked before streaming starts.
struct Reply {
    st: AppState,
    ctx: AuthContext,
    id: String,
    surface: String,
    new_conversation: bool,
    msgs: Vec<Msg>,
    pending: Vec<ToolCall>,
    approved: Option<Vec<String>>,
    auto_approve: bool,
}

/// Streams the reply and saves the conversation when it ends, even if the client went away.
fn stream(reply: Reply, model: HttpModel, first: Option<AssistEvent>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Event, Infallible>>();
    let send = move |e: AssistEvent| {
        if let Ok(ev) = Event::default().event(e.name()).json_data(&e) {
            let _ = tx.send(Ok(ev));
        }
    };
    tokio::spawn(async move {
        if let Some(e) = first {
            send(e);
        }
        let Reply { st, ctx, id, surface, new_conversation, mut msgs, pending, approved, auto_approve } = reply;
        let settings = load_settings(&st, &ctx).await.unwrap_or_default();
        let system = system_prompt(&PromptInput {
            user_name: &ctx.name,
            today: &chrono::Utc::now().format("%Y-%m-%d").to_string(),
            context: &settings.context,
            can_write: tools::may_write(ctx.access),
        });
        let org_tools = OrgTools { st: st.clone(), ctx: ctx.clone(), disabled: settings.disabled_skills };
        let l = Loop { model: &model, tools: &org_tools, system: &system, auto_approve, max_steps: crate::assist::agent::MAX_STEPS };
        let mut usage = Usage::default();
        let calls_before = count_calls(&msgs);
        let mut emit = |e: agent::Event| send(e.into());
        let outcome = match approved {
            Some(approved) => l.resume(&mut msgs, &pending, &approved, &mut usage, &mut emit).await,
            None => l.run(&mut msgs, &mut usage, &mut emit).await,
        };
        let waiting = match &outcome {
            Outcome::Paused(calls) => calls.clone(),
            _ => Vec::new(),
        };
        let saved = sqlx::query("UPDATE assist_conversations SET messages = $4, pending = $5, updated_at = now() WHERE org_id = $1 AND user_id = $2::uuid AND id = $3::uuid")
            .bind(&ctx.org_id)
            .bind(&ctx.user_id)
            .bind(&id)
            .bind(sqlx::types::Json(&msgs))
            .bind((!waiting.is_empty()).then_some(sqlx::types::Json(&waiting)))
            .execute(&st.postgres)
            .await;
        let usage_row = sqlx::query("INSERT INTO assist_usage (org_id, user_id, conversation_id, surface, new_conversation, input_tokens, output_tokens, tool_calls) VALUES ($1, $2::uuid, $3::uuid, $4, $5, $6, $7, $8)")
            .bind(&ctx.org_id)
            .bind(&ctx.user_id)
            .bind(&id)
            .bind(&surface)
            .bind(new_conversation)
            .bind(usage.input as i64)
            .bind(usage.output as i64)
            .bind((count_calls(&msgs) - calls_before) as i32)
            .execute(&st.postgres)
            .await;
        if let Err(e) = saved.and(usage_row) {
            tracing::warn!(error = %e, "assist: saving the conversation failed");
            send(AssistEvent::Error { message: "The answer couldn't be saved".into() });
        }
        if let Outcome::Failed(message) = &outcome {
            send(AssistEvent::Error { message: message.clone() });
        }
        send(AssistEvent::Done { awaiting_approval: !waiting.is_empty(), input_tokens: usage.input, output_tokens: usage.output });
    });
    Sse::new(tokio_stream::wrappers::UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

fn count_calls(msgs: &[Msg]) -> usize {
    msgs.iter().map(|m| if let Msg::Assistant { calls, .. } = m { calls.len() } else { 0 }).sum()
}

async fn configured(st: &AppState, org: &str, user: &str) -> Result<HttpModel, ApiError> {
    crate::routes::assist_usage::check_credits(st, org, Some(user)).await?;
    let r = resolve(st, org)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Assist has no model: an admin sets one in Organization Settings → Assist AI (or the install's OC_ASSIST_PROVIDER).".into()))?;
    let max_tokens = st.config.assist_max_tokens.unwrap_or(match r.provider {
        Provider::Anthropic { .. } => 64_000,
        Provider::OpenAi { .. } => 4_096,
    });
    Ok(HttpModel { provider: r.provider, max_tokens, reach: r.reach })
}

/// Sends a message and streams the reply (`conversation`, `text`, `tool_start` / `tool_end`, `approval`,
/// `error`, then `done`).
#[utoipa::path(post, path = "/api/v1/assist/chat", operation_id = "assist_chat", request_body = ChatRequest,
    responses((status = 200, body = AssistEvent, content_type = "text/event-stream")), tag = "assist")]
pub async fn chat(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<ChatRequest>) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let model = configured(&st, &ctx.org_id, &ctx.user_id).await?;
    let message = req.message.trim().to_string();
    if message.is_empty() || message.chars().count() > MAX_MESSAGE {
        return Err(ApiError::BadRequest(format!("A message has 1 to {MAX_MESSAGE} characters")));
    }
    let surface = match req.surface.as_deref() {
        None | Some("chat") => "chat",
        Some("panel") => "panel",
        Some(other) => return Err(ApiError::BadRequest(format!("Unknown surface {other}"))),
    }
    .to_string();
    let (id, title, mut msgs, new_conversation) = match &req.conversation_id {
        Some(id) => {
            let row = load(&st, &ctx, id).await?;
            if row.pending.is_some_and(|p| !p.0.is_empty()) {
                return Err(ApiError::Conflict("Approve or decline the pending actions first".into()));
            }
            (id.clone(), row.title, row.messages.0, false)
        }
        None => {
            let title = title_of(&message);
            let id: String = sqlx::query_scalar("INSERT INTO assist_conversations (org_id, user_id, title, surface) VALUES ($1, $2::uuid, $3, $4) RETURNING id::text")
                .bind(&ctx.org_id)
                .bind(&ctx.user_id)
                .bind(&title)
                .bind(&surface)
                .fetch_one(&st.postgres)
                .await?;
            (id, title, Vec::new(), true)
        }
    };
    msgs.push(Msg::User(message));
    let first = AssistEvent::Conversation { id: id.clone(), title };
    let reply = Reply { st, ctx, id, surface, new_conversation, msgs, pending: Vec::new(), approved: None, auto_approve: req.auto_approve };
    Ok(stream(reply, model, Some(first)))
}

/// Runs the pending calls the user approved (declining the others) and streams the rest of the reply.
#[utoipa::path(post, path = "/api/v1/assist/conversations/{id}/approve", operation_id = "assist_approve", params(("id" = String, Path)), request_body = ApproveRequest,
    responses((status = 200, body = AssistEvent, content_type = "text/event-stream")), tag = "assist")]
pub async fn approve(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(req): Json<ApproveRequest>) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let model = configured(&st, &ctx.org_id, &ctx.user_id).await?;
    let row = load(&st, &ctx, &id).await?;
    let pending = row.pending.map(|p| p.0).unwrap_or_default();
    if pending.is_empty() {
        return Err(ApiError::Conflict("Nothing is waiting for approval".into()));
    }
    let reply = Reply { st, ctx, id, surface: row.surface, new_conversation: false, msgs: row.messages.0, pending, approved: Some(req.approved), auto_approve: req.auto_approve };
    Ok(stream(reply, model, None))
}

// ---------------------------------------------------------------- reports

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistReportRequest {
    pub range: TimeRange,
    pub user_id: Option<String>,
    pub surface: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistDayCount {
    /// Epoch ms of the day (UTC) or week start.
    pub at: i64,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistSurfaceCount {
    pub surface: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistReport {
    pub total_users: i64,
    pub total_messages: i64,
    pub total_conversations: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub weekly_active_users: Vec<AssistDayCount>,
    pub messages_per_day: Vec<AssistDayCount>,
    pub conversations_per_day: Vec<AssistDayCount>,
    pub messages_by_surface: Vec<AssistSurfaceCount>,
}

/// The report's rows: the org's, in range, optionally of one user and surface ($1…$5).
const REPORT_FILTER: &str = "org_id = $1 AND at >= to_timestamp($2::double precision / 1000) AND at < to_timestamp($3::double precision / 1000) \
     AND ($4::text IS NULL OR user_id::text = $4) AND ($5::text IS NULL OR surface = $5)";

struct ReportFilter<'a> {
    org: &'a str,
    from: i64,
    to: i64,
    user: Option<&'a str>,
    surface: Option<&'a str>,
}

impl<'a> ReportFilter<'a> {
    fn bind<'q, O>(&self, q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>
    where
        'a: 'q,
    {
        q.bind(self.org).bind(self.from).bind(self.to).bind(self.user).bind(self.surface)
    }
}

/// Usage of Assist in the org (Assist → Reports).
#[utoipa::path(post, path = "/api/v1/assist/reports", operation_id = "assist_report", request_body = AssistReportRequest, responses((status = 200, body = AssistReport)), tag = "assist")]
pub async fn report(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AssistReportRequest>) -> ApiResult<AssistReport> {
    req.range.validate_within(400)?;
    let f = ReportFilter { org: &ctx.org_id, from: req.range.from, to: req.range.to, user: req.user_id.as_deref(), surface: req.surface.as_deref() };
    let per = |unit: &str, what: &str| format!("SELECT (extract(epoch FROM date_trunc('{unit}', at)) * 1000)::bigint AS t, {what}::bigint FROM assist_usage WHERE {REPORT_FILTER} GROUP BY t ORDER BY t");
    let totals: (i64, i64, i64, i64, i64) = f.bind(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT count(DISTINCT user_id), count(*), count(*) FILTER (WHERE new_conversation), coalesce(sum(input_tokens), 0)::bigint, coalesce(sum(output_tokens), 0)::bigint FROM assist_usage WHERE {REPORT_FILTER}"
    ))))
    .fetch_one(&st.postgres)
    .await?;
    let surfaces: Vec<(String, i64)> = f.bind(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT surface, count(*) FROM assist_usage WHERE {REPORT_FILTER} GROUP BY surface ORDER BY 2 DESC")))).fetch_all(&st.postgres).await?;
    let series = |sql: String| {
        let f = &f;
        let pg = &st.postgres;
        async move { Ok::<_, ApiError>(f.bind(sqlx::query_as::<_, (i64, i64)>(sqlx::AssertSqlSafe(sql))).fetch_all(pg).await?.into_iter().map(|(at, count)| AssistDayCount { at, count }).collect::<Vec<_>>()) }
    };
    Ok(Json(AssistReport {
        total_users: totals.0,
        total_messages: totals.1,
        total_conversations: totals.2,
        input_tokens: totals.3,
        output_tokens: totals.4,
        weekly_active_users: series(per("week", "count(DISTINCT user_id)")).await?,
        messages_per_day: series(per("day", "count(*)")).await?,
        conversations_per_day: series(per("day", "count(*) FILTER (WHERE new_conversation)")).await?,
        messages_by_surface: surfaces.into_iter().map(|(surface, count)| AssistSurfaceCount { surface, count }).collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::llm::ToolResult;
    use crate::assist::testing::{events, fake_anthropic, sse_text, sse_tool};
    use serde_json::json;

    fn call(id: &str) -> ToolCall {
        ToolCall { id: id.into(), name: "search_logs".into(), input: Ok(json!({"query": "status:error"})) }
    }

    #[test]
    fn a_reply_of_several_turns_shows_as_one_answer_with_its_tools() {
        let msgs = vec![
            Msg::User("Why is checkout failing?".into()),
            Msg::Assistant { text: "Let me look.".into(), calls: vec![call("a"), call("b")], blocks: None },
            Msg::Results(vec![ToolResult { id: "a".into(), content: "42 logs\n…".into(), is_error: false }, ToolResult { id: "b".into(), content: "Invalid query".into(), is_error: true }]),
            Msg::Assistant { text: "Payments time out.".into(), calls: vec![], blocks: None },
            Msg::User("Create a monitor".into()),
            Msg::Assistant { text: String::new(), calls: vec![call("c")], blocks: None },
        ];
        let v = view(&msgs, &[call("c")]);
        assert_eq!(v.len(), 4);
        assert_eq!((v[1].role.as_str(), v[1].text.as_str()), ("assistant", "Let me look.\n\nPayments time out."));
        assert_eq!(v[1].tools.iter().map(|t| (t.status.as_str(), t.summary.as_str())).collect::<Vec<_>>(), vec![("ok", "42 logs"), ("error", "Invalid query")]);
        assert_eq!(v[3].tools[0].status, "pending");
    }

    #[test]
    fn a_conversation_is_titled_by_its_first_line() {
        assert_eq!(title_of("\n  Why is checkout slow?\nDetails…"), "Why is checkout slow?");
        assert_eq!(title_of(&"x".repeat(200)).chars().count(), 80);
    }

    /// The whole loop against a scripted model: a tool answered, a write paused for approval and
    /// run once approved, the conversation saved, usage reported, and only its owner sees it.
    /// Skipped unless OC_TEST_POSTGRES_URL and OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn a_conversation_reads_data_asks_before_writing_and_is_saved() {
        let monitor = json!({"name": "Assist checkout errors", "type": "log", "query": "logs(\"service:checkout status:error\") by {service}", "comparator": "above", "critical": 10});
        let (base_url, seen) = fake_anthropic(vec![
            sse_tool("toolu_1", "search_monitors", json!({"query": "checkout"})),
            sse_text("No checkout monitor exists yet."),
            sse_tool("toolu_2", "create_monitor", monitor),
            sse_text("Created the monitor."),
        ])
        .await;
        let Some((_, states)) = crate::test_support::replicas_with(1, |cfg| {
            cfg.assist = Ok(Some(Provider::Anthropic { base_url: base_url.clone(), api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }));
        })
        .await
        else {
            return;
        };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "assist").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let ctx = AuthContext { user_id, ..admin };
        let ask = |conversation_id: Option<String>, message: &str| ChatRequest { conversation_id, message: message.into(), surface: Some("chat".into()), auto_approve: false };

        // A question answered from a tool.
        let ev = events(chat(State(st.clone()), ctx.clone(), Json(ask(None, "Do we monitor checkout errors?"))).await.unwrap()).await;
        let names: Vec<&str> = ev.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(names, vec!["conversation", "tool_start", "tool_end", "text", "done"], "{ev:?}");
        assert_eq!(ev[2].1["ok"], true, "{ev:?}");
        let id = ev[0].1["id"].as_str().unwrap().to_string();
        assert_eq!(ev[0].1["title"], "Do we monitor checkout errors?");
        let first = seen.lock().unwrap()[0].clone();
        assert_eq!((first["model"].as_str(), first["fallbacks"].as_str()), (Some("claude-opus-5"), Some("default")));
        assert!(first["system"].as_str().unwrap().contains("Test Admin"));

        // A write waits for approval; nothing is created yet.
        let ev = events(chat(State(st.clone()), ctx.clone(), Json(ask(Some(id.clone()), "Create one"))).await.unwrap()).await;
        assert_eq!(ev.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), vec!["conversation", "approval", "done"]);
        assert_eq!(ev[2].1["awaiting_approval"], true);
        let view = get_conversation(State(st.clone()), ctx.clone(), Path(id.clone())).await.unwrap().0;
        assert!(view.awaiting_approval && view.messages[3].tools[0].status == "pending");
        assert!(matches!(chat(State(st.clone()), ctx.clone(), Json(ask(Some(id.clone()), "hello"))).await, Err(ApiError::Conflict(_))));

        // Approved: the monitor exists and the reply continues.
        let ev = events(approve(State(st.clone()), ctx.clone(), Path(id.clone()), Json(ApproveRequest { approved: vec!["toolu_2".into()], auto_approve: false })).await.unwrap()).await;
        assert_eq!(ev.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), vec!["tool_start", "tool_end", "text", "done"], "{ev:?}");
        assert_eq!(ev[1].1["ok"], true, "{ev:?}");
        let made: i64 = sqlx::query_scalar("SELECT count(*) FROM monitors WHERE org_id = $1 AND name = 'Assist checkout errors'").bind(&ctx.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(made, 1);
        let view = get_conversation(State(st.clone()), ctx.clone(), Path(id.clone())).await.unwrap().0;
        assert!(!view.awaiting_approval);
        assert_eq!(view.messages.last().unwrap().text, "Created the monitor.");

        // Listed, reported, and private to its owner.
        assert_eq!(list_conversations(State(st.clone()), ctx.clone()).await.unwrap().0.len(), 1);
        let now = chrono::Utc::now().timestamp_millis();
        let r = report(State(st.clone()), ctx.clone(), Json(AssistReportRequest { range: TimeRange { from: now - 3_600_000, to: now + 60_000 }, user_id: None, surface: None })).await.unwrap().0;
        assert_eq!((r.total_users, r.total_messages, r.total_conversations), (1, 3, 1));
        assert_eq!(r.messages_by_surface[0].surface, "chat");
        let other = AuthContext { user_id: "00000000-0000-4000-8000-0000000000ff".into(), ..ctx.clone() };
        assert!(matches!(get_conversation(State(st.clone()), other, Path(id)).await, Err(ApiError::NotFound(_))));
    }

    #[test]
    fn events_are_named_by_their_type() {
        let e = AssistEvent::Done { awaiting_approval: true, input_tokens: 1, output_tokens: 2 };
        assert_eq!(e.name(), "done");
        assert_eq!(serde_json::to_value(&e).unwrap(), json!({"type": "done", "awaiting_approval": true, "input_tokens": 1, "output_tokens": 2}));
    }
}
