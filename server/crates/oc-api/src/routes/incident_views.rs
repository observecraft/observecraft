//! Incident detail tabs beyond the overview: response team, pages, stakeholder notifications,
//! linked documents, postmortem, and flagging timeline entries as important.
//!
//! Pages and notifications are recorded and shown with their status; delivering them (on-call
//! paging, email, chat) needs an outbound integration, which isn't configured yet.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::incidents::{find, user_name};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

fn text(s: &str, what: &str, max: usize) -> Result<String, ApiError> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(ApiError::BadRequest(format!("{what} must be 1–{max} characters")));
    }
    Ok(t.to_string())
}

async fn event(st: &AppState, pk: i64, kind: &str, ctx: &AuthContext, body: &str) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO incident_events (incident_id, kind, author, author_id, body) VALUES ($1, $2, $3, $4::uuid, $5)")
        .bind(pk)
        .bind(kind)
        .bind(&ctx.name)
        .bind(&ctx.user_id)
        .bind(body)
        .execute(&st.postgres)
        .await?;
    Ok(())
}

// ================================================================ timeline importance

#[derive(Deserialize, ToSchema)]
pub struct ImportantPatch {
    pub important: bool,
}

#[utoipa::path(patch, path = "/api/v1/incidents/{id}/timeline/{event_id}", params(("id" = i32, Path), ("event_id" = i64, Path)), request_body = ImportantPatch, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn set_important(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, Path((id, event_id)): Path<(i32, i64)>, Json(p): Json<ImportantPatch>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &org, id).await?;
    let n = sqlx::query("UPDATE incident_events SET important = $3 WHERE incident_id = $1 AND id = $2").bind(row.pk).bind(event_id).bind(p.important).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Timeline entry not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ response team

/// responder | communications_lead | scribe (the commander is set on the incident itself).
const ROLES: &[&str] = &["responder", "communications_lead", "scribe"];

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub user_id: String,
    pub name: String,
    /// incident_commander | responder | communications_lead | scribe
    pub role: String,
    pub added_at: i64,
    /// Last timeline entry they wrote on this incident.
    pub last_active_at: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/incidents/{id}/responders", params(("id" = i32, Path)), responses((status = 200, body = Vec<TeamMember>)), tag = "incidents")]
pub async fn responders(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<Vec<TeamMember>> {
    let row = find(&st, &org, id).await?;
    let rows = sqlx::query_as(
        "WITH team AS ( \
           SELECT i.commander_id AS user_id, 'incident_commander' AS role, i.declared_at AS added_at FROM incidents i WHERE i.id = $1 AND i.commander_id IS NOT NULL \
           UNION ALL SELECT r.user_id, r.role, r.added_at FROM incident_responders r JOIN incidents i ON i.id = r.incident_id \
           WHERE r.incident_id = $1 AND r.user_id IS DISTINCT FROM i.commander_id) \
         SELECT t.user_id::text AS user_id, u.name, t.role, (extract(epoch FROM t.added_at) * 1000)::bigint AS added_at, \
           (SELECT (extract(epoch FROM max(e.created_at)) * 1000)::bigint FROM incident_events e WHERE e.incident_id = $1 AND e.author_id = t.user_id) AS last_active_at \
         FROM team t JOIN users u ON u.id = t.user_id ORDER BY t.role = 'incident_commander' DESC, t.added_at",
    )
    .bind(row.pk)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddResponder {
    pub user_id: String,
    pub role: String,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/responders", params(("id" = i32, Path)), request_body = AddResponder, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn add_responder(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i32>, Json(req): Json<AddResponder>) -> Result<StatusCode, ApiError> {
    if !ROLES.contains(&req.role.as_str()) {
        return Err(ApiError::BadRequest(format!("role must be one of {}", ROLES.join(", "))));
    }
    let row = find(&st, &ctx.org_id, id).await?;
    let exists: Option<(String,)> = sqlx::query_as("SELECT name FROM users WHERE org_id = $1 AND id::text = $2 AND status = 'active'").bind(&ctx.org_id).bind(&req.user_id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = exists else { return Err(ApiError::BadRequest("unknown or inactive user".into())) };
    sqlx::query("INSERT INTO incident_responders (incident_id, user_id, role) VALUES ($1, $2::uuid, $3) ON CONFLICT (incident_id, user_id) DO UPDATE SET role = EXCLUDED.role")
        .bind(row.pk)
        .bind(&req.user_id)
        .bind(&req.role)
        .execute(&st.postgres)
        .await?;
    event(&st, row.pk, "note", &ctx, &format!("Added {name} to the response team as {}", req.role.replace('_', " "))).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/incidents/{id}/responders/{user_id}", params(("id" = i32, Path), ("user_id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn remove_responder(State(st): State<AppState>, Writer(ctx): Writer, Path((id, user_id)): Path<(i32, String)>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &ctx.org_id, id).await?;
    let n = sqlx::query("DELETE FROM incident_responders WHERE incident_id = $1 AND user_id::text = $2").bind(row.pk).bind(&user_id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Not on the response team".into()));
    }
    let name = user_name(&st, &user_id).await?;
    event(&st, row.pk, "note", &ctx, &format!("Removed {name} from the response team")).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ pages

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: i64,
    pub team: String,
    pub title: String,
    /// triggered | acknowledged | resolved
    pub status: String,
    pub created_by: String,
    pub created_at: i64,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<i64>,
    pub resolved_at: Option<i64>,
}

const PAGE_SELECT: &str = "SELECT id, team_handle AS team, title, status, created_by, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, acknowledged_by, \
    (extract(epoch FROM acknowledged_at) * 1000)::bigint AS acknowledged_at, (extract(epoch FROM resolved_at) * 1000)::bigint AS resolved_at FROM incident_pages";

#[utoipa::path(get, path = "/api/v1/incidents/{id}/pages", params(("id" = i32, Path)), responses((status = 200, body = Vec<Page>)), tag = "incidents")]
pub async fn pages(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<Vec<Page>> {
    let row = find(&st, &org, id).await?;
    let rows = sqlx::query_as(&format!("{PAGE_SELECT} WHERE incident_id = $1 ORDER BY created_at DESC")).bind(row.pk).fetch_all(&st.postgres).await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreatePage {
    pub team: String,
    pub title: String,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/pages", params(("id" = i32, Path)), request_body = CreatePage, responses((status = 200, body = Page)), tag = "incidents")]
pub async fn create_page(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i32>, Json(req): Json<CreatePage>) -> ApiResult<Page> {
    let row = find(&st, &ctx.org_id, id).await?;
    let title = text(&req.title, "title", 300)?;
    let team: Option<(String,)> = sqlx::query_as("SELECT handle FROM teams WHERE org_id = $1 AND handle = $2").bind(&ctx.org_id).bind(&req.team).fetch_optional(&st.postgres).await?;
    let Some((team,)) = team else { return Err(ApiError::BadRequest(format!("unknown team '{}'", req.team))) };
    let (pid,): (i64,) = sqlx::query_as("INSERT INTO incident_pages (incident_id, team_handle, title, created_by) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(row.pk)
        .bind(&team)
        .bind(&title)
        .bind(&ctx.name)
        .fetch_one(&st.postgres)
        .await?;
    event(&st, row.pk, "notification", &ctx, &format!("Paged @{team}: {title}")).await?;
    let page = sqlx::query_as(&format!("{PAGE_SELECT} WHERE id = $1")).bind(pid).fetch_one(&st.postgres).await?;
    Ok(Json(page))
}

#[derive(Deserialize, ToSchema)]
pub struct PagePatch {
    /// acknowledged | resolved
    pub status: String,
}

#[utoipa::path(patch, path = "/api/v1/incidents/{id}/pages/{page_id}", params(("id" = i32, Path), ("page_id" = i64, Path)), request_body = PagePatch, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn update_page(State(st): State<AppState>, Writer(ctx): Writer, Path((id, page_id)): Path<(i32, i64)>, Json(p): Json<PagePatch>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &ctx.org_id, id).await?;
    let sql = match p.status.as_str() {
        "acknowledged" => "UPDATE incident_pages SET status = 'acknowledged', acknowledged_by = $3, acknowledged_at = now() WHERE incident_id = $1 AND id = $2 AND status = 'triggered'",
        "resolved" => "UPDATE incident_pages SET status = 'resolved', resolved_at = now(), acknowledged_by = COALESCE(acknowledged_by, $3), acknowledged_at = COALESCE(acknowledged_at, now()) WHERE incident_id = $1 AND id = $2 AND status <> 'resolved'",
        _ => return Err(ApiError::BadRequest("status must be acknowledged or resolved".into())),
    };
    let n = sqlx::query(sql).bind(row.pk).bind(page_id).bind(&ctx.name).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Page not found or already in that state".into()));
    }
    event(&st, row.pk, "note", &ctx, &format!("Page {} {}", page_id, p.status)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ notifications

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: i64,
    pub recipients: Vec<String>,
    pub subject: String,
    pub body: String,
    pub automated: bool,
    pub author: String,
    pub created_at: i64,
}

#[utoipa::path(get, path = "/api/v1/incidents/{id}/notifications", params(("id" = i32, Path)), responses((status = 200, body = Vec<Notification>)), tag = "incidents")]
pub async fn notifications(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<Vec<Notification>> {
    let row = find(&st, &org, id).await?;
    let rows = sqlx::query_as(
        "SELECT id, recipients, subject, body, automated, author, (extract(epoch FROM created_at) * 1000)::bigint AS created_at \
         FROM incident_notifications WHERE incident_id = $1 ORDER BY created_at DESC",
    )
    .bind(row.pk)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateNotification {
    /// Email addresses, #channels or @handles.
    pub recipients: Vec<String>,
    pub subject: String,
    #[serde(default)]
    pub body: String,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/notifications", params(("id" = i32, Path)), request_body = CreateNotification, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn create_notification(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i32>, Json(req): Json<CreateNotification>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &ctx.org_id, id).await?;
    let recipients: Vec<String> = req.recipients.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    if recipients.is_empty() || recipients.len() > 50 || recipients.iter().any(|r| r.len() > 254 || r.contains(char::is_whitespace)) {
        return Err(ApiError::BadRequest("add 1–50 recipients (emails, #channels or @handles)".into()));
    }
    let subject = text(&req.subject, "subject", 300)?;
    let body: String = req.body.trim().chars().take(5000).collect();
    sqlx::query("INSERT INTO incident_notifications (incident_id, recipients, subject, body, automated, author) VALUES ($1, $2, $3, $4, false, $5)")
        .bind(row.pk)
        .bind(&recipients)
        .bind(&subject)
        .bind(&body)
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?;
    event(&st, row.pk, "notification", &ctx, &format!("{subject} — sent to {}", recipients.join(", "))).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ documents

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub created_by: String,
    pub created_at: i64,
}

#[utoipa::path(get, path = "/api/v1/incidents/{id}/documents", params(("id" = i32, Path)), responses((status = 200, body = Vec<Document>)), tag = "incidents")]
pub async fn documents(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<Vec<Document>> {
    let row = find(&st, &org, id).await?;
    let rows = sqlx::query_as("SELECT id, title, url, created_by, (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM incident_documents WHERE incident_id = $1 ORDER BY created_at")
        .bind(row.pk)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateDocument {
    pub title: String,
    pub url: String,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/documents", params(("id" = i32, Path)), request_body = CreateDocument, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn add_document(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i32>, Json(req): Json<CreateDocument>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &ctx.org_id, id).await?;
    let title = text(&req.title, "title", 200)?;
    // Only web links: these render as anchors in the UI.
    let url = url::Url::parse(req.url.trim()).ok().filter(|u| matches!(u.scheme(), "http" | "https")).ok_or_else(|| ApiError::BadRequest("link must be an http(s) URL".into()))?;
    sqlx::query("INSERT INTO incident_documents (incident_id, title, url, created_by) VALUES ($1, $2, $3, $4)").bind(row.pk).bind(&title).bind(url.as_str()).bind(&ctx.name).execute(&st.postgres).await?;
    event(&st, row.pk, "note", &ctx, &format!("Linked document \"{title}\": {url}")).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/incidents/{id}/documents/{doc_id}", params(("id" = i32, Path), ("doc_id" = i64, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn remove_document(State(st): State<AppState>, Writer(ctx): Writer, Path((id, doc_id)): Path<(i32, i64)>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &ctx.org_id, id).await?;
    let n = sqlx::query("DELETE FROM incident_documents WHERE incident_id = $1 AND id = $2").bind(row.pk).bind(doc_id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Document not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ postmortem

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Postmortem {
    pub template: String,
    /// Markdown.
    pub content: String,
    pub created_by: String,
    pub updated_at: i64,
}

#[utoipa::path(get, path = "/api/v1/incidents/{id}/postmortem", params(("id" = i32, Path)), responses((status = 200, body = Option<Postmortem>)), tag = "incidents")]
pub async fn postmortem(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<Option<Postmortem>> {
    let row = find(&st, &org, id).await?;
    let pm: Option<(String, String, String, i64)> = sqlx::query_as("SELECT template, content, created_by, (extract(epoch FROM updated_at) * 1000)::bigint FROM incident_postmortems WHERE incident_id = $1")
        .bind(row.pk)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(Json(pm.map(|(template, content, created_by, updated_at)| Postmortem { template, content, created_by, updated_at })))
}

#[derive(Deserialize, ToSchema)]
pub struct GeneratePostmortem {
    /// standard | brief
    pub template: String,
}

fn fmt_time(ms: i64) -> String {
    let secs = ms / 1000;
    let days = secs.div_euclid(86_400);
    let (h, m) = (secs.rem_euclid(86_400) / 3600, secs.rem_euclid(3600) / 60);
    // Civil date from days since epoch (Howard Hinnant's algorithm), UTC.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

fn fmt_duration(ms: i64) -> String {
    let m = ms / 60_000;
    match m {
        m if m < 60 => format!("{m} min"),
        m if m < 1440 => format!("{}h {}m", m / 60, m % 60),
        m => format!("{}d {}h", m / 1440, (m % 1440) / 60),
    }
}

/// Fills a postmortem from the incident: properties, impact, root cause, timeline and follow-ups. Editable afterwards.
#[utoipa::path(post, path = "/api/v1/incidents/{id}/postmortem", params(("id" = i32, Path)), request_body = GeneratePostmortem, responses((status = 200, body = Postmortem)), tag = "incidents")]
pub async fn generate_postmortem(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i32>, Json(req): Json<GeneratePostmortem>) -> ApiResult<Postmortem> {
    let brief = match req.template.as_str() {
        "standard" => false,
        "brief" => true,
        _ => return Err(ApiError::BadRequest("template must be standard or brief".into())),
    };
    let r = find(&st, &ctx.org_id, id).await?;
    let events: Vec<(String, String, String, i64, bool)> = sqlx::query_as("SELECT kind, author, body, (extract(epoch FROM created_at) * 1000)::bigint, important FROM incident_events WHERE incident_id = $1 ORDER BY created_at, id")
        .bind(r.pk)
        .fetch_all(&st.postgres)
        .await?;
    let impacts: Vec<(String,)> = sqlx::query_as("SELECT description FROM incident_impacts WHERE incident_id = $1 ORDER BY start_at").bind(r.pk).fetch_all(&st.postgres).await?;
    let followups: Vec<(String, bool)> = sqlx::query_as("SELECT title, done FROM incident_followups WHERE incident_id = $1 ORDER BY id").bind(r.pk).fetch_all(&st.postgres).await?;
    let docs: Vec<(String, String)> = sqlx::query_as("SELECT title, url FROM incident_documents WHERE incident_id = $1 ORDER BY id").bind(r.pk).fetch_all(&st.postgres).await?;

    let todo = "_To be completed._";
    let or = |v: &str| if v.trim().is_empty() { todo.to_string() } else { v.trim().to_string() };
    let end = r.resolved_at.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64);
    let mut md = format!("# Postmortem IR-{}: {}\n\n## Overview\n\n| | |\n|---|---|\n", r.public_id, r.title);
    for (k, v) in [
        ("Status", r.status.clone()),
        ("Severity", r.severity.clone()),
        ("Type", r.incident_type.clone()),
        ("Commander", r.commander_name.clone().unwrap_or_else(|| "Unassigned".into())),
        ("Detected", fmt_time(r.detected_at)),
        ("Declared", fmt_time(r.declared_at)),
        ("Resolved", r.resolved_at.map(fmt_time).unwrap_or_else(|| "Not resolved".into())),
        ("Duration", fmt_duration(end - r.declared_at)),
        ("Services", if r.services.is_empty() { "—".into() } else { r.services.join(", ") }),
        ("Teams", if r.teams.is_empty() { "—".into() } else { r.teams.join(", ") }),
        ("Customer impact", if r.customer_impact { "Yes".into() } else { "No".into() }),
    ] {
        md.push_str(&format!("| {k} | {} |\n", v.replace('|', "\\|")));
    }
    md.push_str(&format!("\n{}\n\n## What Happened?\n\n### Impact on Customers\n\n", or(&r.summary)));
    if impacts.is_empty() {
        md.push_str(if r.customer_impact { "_Customers were impacted; describe how._\n" } else { "No customer impact recorded.\n" });
    } else {
        for (d,) in &impacts {
            md.push_str(&format!("- {d}\n"));
        }
    }
    md.push_str(&format!("\n## Why Did it Happen?\n\n### Root Cause\n\n{}\n", or(&r.root_cause)));
    if !brief {
        md.push_str("\n## Timeline\n\n");
        for (kind, author, body, at, important) in &events {
            let star = if *important { " ⭐" } else { "" };
            md.push_str(&format!("- **{}** — {author} ({kind}){star}: {}\n", fmt_time(*at), body.replace('\n', " ")));
        }
    }
    md.push_str("\n## How do we prevent it in the future?\n\n### Action Items\n\n");
    if followups.is_empty() {
        md.push_str("_No follow-ups yet._\n");
    } else {
        for (t, done) in &followups {
            md.push_str(&format!("- [{}] {t}\n", if *done { "x" } else { " " }));
        }
    }
    if !brief {
        md.push_str(&format!("\n## Lessons Learned\n\n{todo}\n"));
        if !docs.is_empty() {
            md.push_str("\n## Related Documents\n\n");
            for (t, u) in &docs {
                md.push_str(&format!("- [{t}]({u})\n"));
            }
        }
    }
    let template = if brief { "brief" } else { "standard" };
    sqlx::query(
        "INSERT INTO incident_postmortems (incident_id, template, content, created_by) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (incident_id) DO UPDATE SET template = EXCLUDED.template, content = EXCLUDED.content, updated_at = now()",
    )
    .bind(r.pk)
    .bind(template)
    .bind(&md)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await?;
    event(&st, r.pk, "note", &ctx, "Generated the postmortem").await?;
    Ok(Json(Postmortem { template: template.into(), content: md, created_by: ctx.name.clone(), updated_at: end }))
}

#[derive(Deserialize, ToSchema)]
pub struct PostmortemUpdate {
    pub content: String,
}

#[utoipa::path(put, path = "/api/v1/incidents/{id}/postmortem", params(("id" = i32, Path)), request_body = PostmortemUpdate, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn update_postmortem(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i32>, Json(req): Json<PostmortemUpdate>) -> Result<StatusCode, ApiError> {
    if req.content.len() > 200_000 {
        return Err(ApiError::BadRequest("postmortem is too long".into()));
    }
    let r = find(&st, &ctx.org_id, id).await?;
    let n = sqlx::query("UPDATE incident_postmortems SET content = $2, updated_at = now() WHERE incident_id = $1").bind(r.pk).bind(&req.content).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Generate the postmortem first".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::{fmt_duration, fmt_time};

    #[test]
    fn formats() {
        assert_eq!(fmt_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(fmt_time(1_792_929_600_000), "2026-10-25 12:00 UTC");
        assert_eq!(fmt_duration(90 * 60_000), "1h 30m");
        assert_eq!(fmt_duration(26 * 3_600_000), "1d 2h");
    }
}
