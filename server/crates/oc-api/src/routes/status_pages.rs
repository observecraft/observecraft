//! Status pages (Incident Response → Status Pages): the state of an org's components for its users
//! and the notices posted while an incident affects them. Anyone in the org reads them; standard
//! users and admins create and edit pages and post notices.
//!
//! A page's slug is unique across the instance: `/status/<slug>` (the web app) reads it through
//! `GET /api/v1/public/status-pages/{slug}`, which needs no sign-in. That route answers for public
//! pages to anyone and for internal pages only to signed-in members of the page's org; everyone
//! else gets the same 404 as for a slug that doesn't exist.
//!
//! A component's current status is the worst of the status set on the page and the impacts of the
//! notices still open on it; the page's overall status is the worst of its components.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_PAGES: i64 = 100;
const MAX_COMPONENTS: usize = 100;
const MAX_NAME_CHARS: usize = 100;
const MAX_SLUG_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 1000;
const MAX_TITLE_CHARS: usize = 200;
const MAX_MESSAGE_CHARS: usize = 10_000;
/// Notices resolved longer ago than this aren't returned (the uptime bars cover as many days).
const HISTORY_DAYS: i32 = 60;
const MAX_NOTICES: i64 = 500;
const MAX_URL_CHARS: usize = 2048;

/// Ordered from best to worst.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    Operational,
    DegradedPerformance,
    PartialOutage,
    MajorOutage,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusPageVisibility {
    /// Anyone with the link sees it at `/status/<slug>`.
    Public,
    /// Only signed-in members of the org see it.
    Internal,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoticeStatus {
    Investigating,
    Identified,
    Monitoring,
    Resolved,
}

impl StatusPageVisibility {
    fn as_str(self) -> &'static str {
        match self {
            StatusPageVisibility::Public => "public",
            StatusPageVisibility::Internal => "internal",
        }
    }
}

impl NoticeStatus {
    fn as_str(self) -> &'static str {
        match self {
            NoticeStatus::Investigating => "investigating",
            NoticeStatus::Identified => "identified",
            NoticeStatus::Monitoring => "monitoring",
            NoticeStatus::Resolved => "resolved",
        }
    }
}

/// A component as stored in `status_pages.components`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct StoredComponent {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    status: ComponentStatus,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusPageComponentInput {
    /// The id of an existing component to keep (and its notice history); omit for a new one.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    /// Components with the same group are shown together under it.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default = "operational")]
    pub status: ComponentStatus,
}

fn operational() -> ComponentStatus {
    ComponentStatus::Operational
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusPageComponent {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    /// The status set on the page.
    pub status: ComponentStatus,
    /// The worst of `status` and the impacts of the open notices.
    pub current_status: ComponentStatus,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoticeImpact {
    pub component_id: String,
    /// Degraded performance, partial outage or major outage.
    pub impact: ComponentStatus,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NoticeUpdate {
    pub id: i64,
    pub status: NoticeStatus,
    /// Markdown.
    pub message: String,
    /// The impacts from this update on.
    pub impacts: Vec<NoticeImpact>,
    pub created_by: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusPageNotice {
    pub id: String,
    pub title: String,
    /// The status of the latest update.
    pub status: NoticeStatus,
    /// The impacts of the latest update.
    pub impacts: Vec<NoticeImpact>,
    /// The linked incident's IR number.
    pub incident_id: Option<i32>,
    pub created_by: Option<String>,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub resolved_at: Option<i64>,
    /// Newest first; the last one opened the notice.
    pub updates: Vec<NoticeUpdate>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusPageSummary {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub visibility: StatusPageVisibility,
    pub component_count: i64,
    pub overall_status: ComponentStatus,
    /// Notices not resolved yet.
    pub open_notices: i64,
    pub updated_by: Option<String>,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusPageDetail {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub visibility: StatusPageVisibility,
    pub description: String,
    /// Shown in the page's header instead of its name; empty for none.
    pub logo_url: String,
    /// The page offers to subscribe to its updates by email / in Slack.
    pub email_subscriptions: bool,
    pub slack_subscriptions: bool,
    pub components: Vec<StatusPageComponent>,
    pub overall_status: ComponentStatus,
    /// Open notices and those resolved in the last 60 days, newest first.
    pub notices: Vec<StatusPageNotice>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

/// The read-only rendering at `/status/<slug>`: no ids of users, incidents or the org.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicStatusPage {
    pub name: String,
    pub slug: String,
    pub visibility: StatusPageVisibility,
    pub description: String,
    pub logo_url: String,
    pub email_subscriptions: bool,
    pub slack_subscriptions: bool,
    pub components: Vec<StatusPageComponent>,
    pub overall_status: ComponentStatus,
    /// Open notices and those resolved in the last 60 days, newest first, without authors or incident links.
    pub notices: Vec<StatusPageNotice>,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusPageInput {
    pub name: String,
    /// 1–64 lowercase letters, digits and dashes (not first or last); unique across the instance.
    pub slug: String,
    pub visibility: StatusPageVisibility,
    #[serde(default)]
    pub description: String,
    /// An http(s) image URL shown in the header instead of the name; empty for none.
    #[serde(default)]
    pub logo_url: String,
    /// Show a subscribe button offering email / Slack updates (subscriptions aren't delivered yet).
    #[serde(default)]
    pub email_subscriptions: bool,
    #[serde(default)]
    pub slack_subscriptions: bool,
    /// In display order.
    #[serde(default)]
    pub components: Vec<StatusPageComponentInput>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoticeInput {
    pub title: String,
    /// Markdown: the first update.
    pub message: String,
    pub status: NoticeStatus,
    #[serde(default)]
    pub impacts: Vec<NoticeImpact>,
    /// IR number of an incident of the org.
    #[serde(default)]
    pub incident_id: Option<i32>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoticeUpdateInput {
    /// Markdown.
    pub message: String,
    /// `resolved` resolves the notice; no update can follow.
    pub status: NoticeStatus,
    /// New impacts; omit to keep the current ones.
    #[serde(default)]
    pub impacts: Option<Vec<NoticeImpact>>,
}

// ================================================================ status

/// The worst status of the list (operational for none).
fn worst(statuses: impl IntoIterator<Item = ComponentStatus>) -> ComponentStatus {
    statuses.into_iter().max().unwrap_or(ComponentStatus::Operational)
}

/// Components with their current status: the worst of the one set and the open notices' impacts.
/// `open` holds the impacts of every open notice.
fn current_components(components: &[StoredComponent], open: &[&[NoticeImpact]]) -> Vec<StatusPageComponent> {
    let mut impacts: HashMap<&str, ComponentStatus> = HashMap::new();
    for i in open.iter().flat_map(|n| n.iter()) {
        let e = impacts.entry(i.component_id.as_str()).or_insert(ComponentStatus::Operational);
        *e = (*e).max(i.impact);
    }
    components
        .iter()
        .map(|c| StatusPageComponent {
            id: c.id.clone(),
            name: c.name.clone(),
            group: c.group.clone(),
            status: c.status,
            current_status: worst([c.status, impacts.get(c.id.as_str()).copied().unwrap_or(ComponentStatus::Operational)]),
        })
        .collect()
}

// ================================================================ validation

/// Trims a required text and checks its length.
fn required(value: &str, what: &str, max: usize) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("{what} is required"));
    }
    if v.chars().count() > max {
        return Err(format!("{what} is at most {max} characters"));
    }
    Ok(v.to_string())
}

fn check_slug(slug: &str) -> Result<(), String> {
    let b = slug.as_bytes();
    let valid = (1..=MAX_SLUG_CHARS).contains(&b.len()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-') && b[0] != b'-' && b[b.len() - 1] != b'-';
    if valid {
        Ok(())
    } else {
        Err(format!("The slug must be 1–{MAX_SLUG_CHARS} characters: lowercase letters, digits and dashes, not starting or ending with a dash"))
    }
}

/// A new random component id (`c_` and 12 lowercase letters or digits).
fn new_component_id() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let tail: String = (0..12).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect();
    format!("c_{tail}")
}

/// Checks the page's fields and turns its components into stored ones: names trimmed and unique,
/// ids of components in `existing` kept, every other component given a new id.
fn check_page(input: &mut StatusPageInput, existing: &[StoredComponent]) -> Result<Vec<StoredComponent>, String> {
    input.name = required(&input.name, "The name", MAX_NAME_CHARS)?;
    input.slug = input.slug.trim().to_string();
    check_slug(&input.slug)?;
    input.description = input.description.trim().to_string();
    if input.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    input.logo_url = input.logo_url.trim().to_string();
    if !input.logo_url.is_empty() {
        if input.logo_url.chars().count() > MAX_URL_CHARS {
            return Err(format!("The logo URL is at most {MAX_URL_CHARS} characters"));
        }
        if !url::Url::parse(&input.logo_url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
            return Err("The logo must be an http or https URL".into());
        }
    }
    if input.components.len() > MAX_COMPONENTS {
        return Err(format!("A status page has at most {MAX_COMPONENTS} components"));
    }
    let known: HashSet<&str> = existing.iter().map(|c| c.id.as_str()).collect();
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    let mut out = Vec::with_capacity(input.components.len());
    for c in &input.components {
        let name = required(&c.name, "The component name", MAX_NAME_CHARS)?;
        if !names.insert(name.to_lowercase()) {
            return Err(format!("Component '{}' appears twice", name.to_lowercase()));
        }
        let group = c.group.as_deref().map(str::trim).filter(|g| !g.is_empty()).map(str::to_string);
        if group.as_ref().is_some_and(|g| g.chars().count() > MAX_NAME_CHARS) {
            return Err(format!("Group names are at most {MAX_NAME_CHARS} characters"));
        }
        let id = match c.id.as_deref() {
            Some(id) if known.contains(id) && !ids.contains(id) => id.to_string(),
            _ => new_component_id(),
        };
        ids.insert(id.clone());
        out.push(StoredComponent { id, name, group, status: c.status });
    }
    Ok(out)
}

/// Impacts on components of the page, each component once, none of them `operational`.
fn check_impacts(impacts: &[NoticeImpact], components: &[StoredComponent]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for i in impacts {
        let Some(c) = components.iter().find(|c| c.id == i.component_id) else {
            return Err(format!("Component '{}' is not on this status page", i.component_id));
        };
        if !seen.insert(c.id.as_str()) {
            return Err(format!("Component '{}' is impacted twice", c.name));
        }
        if i.impact == ComponentStatus::Operational {
            return Err(format!("The impact on '{}' must be degraded performance, partial outage or major outage", c.name));
        }
    }
    Ok(())
}

/// Path ids are UUIDs; anything else can't name a page or notice.
fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

fn page_not_found() -> ApiError {
    ApiError::NotFound("Status page not found".into())
}

fn notice_not_found() -> ApiError {
    ApiError::NotFound("Notice not found".into())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct PageRow {
    id: String,
    name: String,
    slug: String,
    visibility: String,
    description: String,
    logo_url: String,
    email_subscriptions: bool,
    slack_subscriptions: bool,
    components: Value,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct NoticeRow {
    id: String,
    title: String,
    status: String,
    impacts: Value,
    incident_id: Option<i32>,
    created_by: Option<String>,
    started_at: i64,
    resolved_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct UpdateRow {
    id: i64,
    notice_id: String,
    status: String,
    message: String,
    impacts: Value,
    created_by: Option<String>,
    created_at: i64,
}

fn visibility(s: &str) -> StatusPageVisibility {
    if s == "public" {
        StatusPageVisibility::Public
    } else {
        StatusPageVisibility::Internal
    }
}

fn notice_status(s: &str) -> NoticeStatus {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(NoticeStatus::Investigating)
}

fn stored_components(v: Value) -> Vec<StoredComponent> {
    serde_json::from_value(v).unwrap_or_default()
}

fn impacts(v: Value) -> Vec<NoticeImpact> {
    serde_json::from_value(v).unwrap_or_default()
}

fn json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

/// A timestamp column as epoch milliseconds.
fn ms(column: &str) -> String {
    format!("(extract(epoch FROM {column}) * 1000)::bigint")
}

fn page_select() -> String {
    format!(
        "SELECT p.id::text AS id, p.name, p.slug, p.visibility, p.description, p.logo_url, p.email_subscriptions, p.slack_subscriptions, p.components, c.name AS created_by, u.name AS updated_by, \
         {} AS created_at, {} AS updated_at FROM status_pages p LEFT JOIN users c ON c.id = p.created_by LEFT JOIN users u ON u.id = p.updated_by",
        ms("p.created_at"),
        ms("p.updated_at")
    )
}

async fn page_row(st: &AppState, org: &str, id: &str) -> Result<PageRow, ApiError> {
    if !is_uuid(id) {
        return Err(page_not_found());
    }
    let row: Option<PageRow> = sqlx::query_as(&format!("{} WHERE p.org_id = $1 AND p.id = $2::uuid", page_select())).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.ok_or_else(page_not_found)
}

/// The page's open notices and those resolved in the last 60 days, newest first, with their updates.
async fn notices(st: &AppState, org: &str, page_id: &str) -> Result<Vec<StatusPageNotice>, ApiError> {
    let rows: Vec<NoticeRow> = sqlx::query_as(&format!(
        "SELECT n.id::text AS id, n.title, n.status, n.impacts, n.incident_id, u.name AS created_by, {} AS started_at, {} AS resolved_at \
         FROM status_page_notices n LEFT JOIN users u ON u.id = n.created_by \
         WHERE n.org_id = $1 AND n.page_id = $2::uuid AND (n.resolved_at IS NULL OR n.resolved_at > now() - make_interval(days => $3)) \
         ORDER BY n.started_at DESC LIMIT $4",
        ms("n.started_at"),
        ms("n.resolved_at")
    ))
    .bind(org)
    .bind(page_id)
    .bind(HISTORY_DAYS)
    .bind(MAX_NOTICES)
    .fetch_all(&st.postgres)
    .await?;
    let ids: Vec<String> = rows.iter().map(|n| n.id.clone()).collect();
    let updates: Vec<UpdateRow> = sqlx::query_as(&format!(
        "SELECT x.id, x.notice_id::text AS notice_id, x.status, x.message, x.impacts, u.name AS created_by, {} AS created_at \
         FROM status_page_notice_updates x LEFT JOIN users u ON u.id = x.created_by \
         WHERE x.org_id = $1 AND x.notice_id = ANY($2::text[]::uuid[]) ORDER BY x.created_at DESC, x.id DESC",
        ms("x.created_at")
    ))
    .bind(org)
    .bind(&ids)
    .fetch_all(&st.postgres)
    .await?;
    let mut by_notice: HashMap<String, Vec<NoticeUpdate>> = HashMap::new();
    for u in updates {
        by_notice.entry(u.notice_id).or_default().push(NoticeUpdate {
            id: u.id,
            status: notice_status(&u.status),
            message: u.message,
            impacts: impacts(u.impacts),
            created_by: u.created_by,
            created_at: u.created_at,
        });
    }
    Ok(rows
        .into_iter()
        .map(|n| StatusPageNotice {
            updates: by_notice.remove(&n.id).unwrap_or_default(),
            id: n.id,
            title: n.title,
            status: notice_status(&n.status),
            impacts: impacts(n.impacts),
            incident_id: n.incident_id,
            created_by: n.created_by,
            started_at: n.started_at,
            resolved_at: n.resolved_at,
        })
        .collect())
}

fn detail(page: PageRow, notices: Vec<StatusPageNotice>) -> StatusPageDetail {
    let open: Vec<&[NoticeImpact]> = notices.iter().filter(|n| n.resolved_at.is_none()).map(|n| n.impacts.as_slice()).collect();
    let components = current_components(&stored_components(page.components), &open);
    StatusPageDetail {
        overall_status: worst(components.iter().map(|c| c.current_status)),
        components,
        notices,
        id: page.id,
        name: page.name,
        slug: page.slug,
        visibility: visibility(&page.visibility),
        description: page.description,
        logo_url: page.logo_url,
        email_subscriptions: page.email_subscriptions,
        slack_subscriptions: page.slack_subscriptions,
        created_by: page.created_by,
        updated_by: page.updated_by,
        created_at: page.created_at,
        updated_at: page.updated_at,
    }
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<StatusPageDetail, ApiError> {
    let page = page_row(st, org, id).await?;
    let notices = notices(st, org, &page.id).await?;
    Ok(detail(page, notices))
}

/// The detail as `/status/<slug>` shows it: authors and incident links left out.
fn public(d: StatusPageDetail) -> PublicStatusPage {
    let notices = d
        .notices
        .into_iter()
        .map(|n| StatusPageNotice { incident_id: None, created_by: None, updates: n.updates.into_iter().map(|u| NoticeUpdate { created_by: None, ..u }).collect(), ..n })
        .collect();
    PublicStatusPage {
        name: d.name,
        slug: d.slug,
        visibility: d.visibility,
        description: d.description,
        logo_url: d.logo_url,
        email_subscriptions: d.email_subscriptions,
        slack_subscriptions: d.slack_subscriptions,
        components: d.components,
        overall_status: d.overall_status,
        notices,
        updated_at: d.updated_at,
    }
}

fn slug_taken(e: sqlx::Error, slug: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("The slug '{slug}' is already taken")),
        _ => e.into(),
    }
}

/// The page's components, locked for the rest of the transaction.
async fn lock_components(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: &str, id: &str) -> Result<Vec<StoredComponent>, ApiError> {
    if !is_uuid(id) {
        return Err(page_not_found());
    }
    let row: Option<(Value,)> = sqlx::query_as("SELECT components FROM status_pages WHERE org_id = $1 AND id = $2::uuid FOR UPDATE").bind(org).bind(id).fetch_optional(&mut **tx).await?;
    row.map(|(c,)| stored_components(c)).ok_or_else(page_not_found)
}

/// Notice activity counts as an update of the page.
async fn touch(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, ctx: &AuthContext, id: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE status_pages SET updated_at = now(), updated_by = $3::uuid WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(id).bind(&ctx.user_id).execute(&mut **tx).await?;
    Ok(())
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/status-pages", operation_id = "list_status_pages", responses((status = 200, body = Vec<StatusPageSummary>)), tag = "incidents")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<StatusPageSummary>> {
    let pages: Vec<PageRow> = sqlx::query_as(&format!("{} WHERE p.org_id = $1 ORDER BY lower(p.name), p.slug", page_select())).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let open: Vec<(String, Value)> =
        sqlx::query_as("SELECT page_id::text, impacts FROM status_page_notices WHERE org_id = $1 AND resolved_at IS NULL").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let mut by_page: HashMap<String, Vec<Vec<NoticeImpact>>> = HashMap::new();
    for (page, i) in open {
        by_page.entry(page).or_default().push(impacts(i));
    }
    Ok(Json(
        pages
            .into_iter()
            .map(|p| {
                let notices = by_page.remove(&p.id).unwrap_or_default();
                let open: Vec<&[NoticeImpact]> = notices.iter().map(Vec::as_slice).collect();
                let components = current_components(&stored_components(p.components), &open);
                StatusPageSummary {
                    component_count: components.len() as i64,
                    overall_status: worst(components.iter().map(|c| c.current_status)),
                    open_notices: notices.len() as i64,
                    id: p.id,
                    name: p.name,
                    slug: p.slug,
                    visibility: visibility(&p.visibility),
                    updated_by: p.updated_by,
                    updated_at: p.updated_at,
                }
            })
            .collect(),
    ))
}

#[utoipa::path(post, path = "/api/v1/status-pages", operation_id = "create_status_page", request_body = StatusPageInput,
    responses((status = 200, body = StatusPageDetail), (status = 400, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<StatusPageInput>) -> ApiResult<StatusPageDetail> {
    let components = check_page(&mut input, &[]).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM status_pages WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_PAGES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_PAGES} status pages")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO status_pages (org_id, name, slug, visibility, description, components, created_by, updated_by, logo_url, email_subscriptions, slack_subscriptions) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $7::uuid, $8, $9, $10) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(&input.slug)
    .bind(input.visibility.as_str())
    .bind(&input.description)
    .bind(json(&components))
    .bind(&ctx.user_id)
    .bind(&input.logo_url)
    .bind(input.email_subscriptions)
    .bind(input.slack_subscriptions)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| slug_taken(e, &input.slug))?;
    ctx.audit(&st, "status_page.created", &input.slug, &format!("{} ({}, {} components)", input.name, input.visibility.as_str(), components.len())).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(get, path = "/api/v1/status-pages/{id}", operation_id = "get_status_page", params(("id" = String, Path)),
    responses((status = 200, body = StatusPageDetail), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<StatusPageDetail> {
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// Replaces the page's fields and components. Components sent with the id of an existing one keep
/// it (and its notice history); the others get new ids.
#[utoipa::path(put, path = "/api/v1/status-pages/{id}", operation_id = "update_status_page", params(("id" = String, Path)), request_body = StatusPageInput,
    responses((status = 200, body = StatusPageDetail), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<StatusPageInput>) -> ApiResult<StatusPageDetail> {
    let mut tx = st.postgres.begin().await?;
    let existing = lock_components(&mut tx, &ctx.org_id, &id).await?;
    let components = check_page(&mut input, &existing).map_err(ApiError::BadRequest)?;
    sqlx::query(
        "UPDATE status_pages SET name = $3, slug = $4, visibility = $5, description = $6, components = $7, updated_by = $8::uuid, updated_at = now(), \
         logo_url = $9, email_subscriptions = $10, slack_subscriptions = $11 WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.slug)
    .bind(input.visibility.as_str())
    .bind(&input.description)
    .bind(json(&components))
    .bind(&ctx.user_id)
    .bind(&input.logo_url)
    .bind(input.email_subscriptions)
    .bind(input.slack_subscriptions)
    .execute(&mut *tx)
    .await
    .map_err(|e| slug_taken(e, &input.slug))?;
    tx.commit().await?;
    ctx.audit(&st, "status_page.updated", &input.slug, &format!("{} ({}, {} components)", input.name, input.visibility.as_str(), components.len())).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/status-pages/{id}", operation_id = "delete_status_page", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) {
        return Err(page_not_found());
    }
    let row: Option<(String, String)> =
        sqlx::query_as("DELETE FROM status_pages WHERE org_id = $1 AND id = $2::uuid RETURNING slug, name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((slug, name)) = row else { return Err(page_not_found()) };
    ctx.audit(&st, "status_page.deleted", &slug, &name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Opens a notice with its first update.
#[utoipa::path(post, path = "/api/v1/status-pages/{id}/notices", operation_id = "create_status_page_notice", params(("id" = String, Path)), request_body = NoticeInput,
    responses((status = 200, body = StatusPageDetail), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn create_notice(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<NoticeInput>) -> ApiResult<StatusPageDetail> {
    let title = required(&input.title, "The title", MAX_TITLE_CHARS).map_err(ApiError::BadRequest)?;
    let message = required(&input.message, "The message", MAX_MESSAGE_CHARS).map_err(ApiError::BadRequest)?;
    let mut tx = st.postgres.begin().await?;
    let components = lock_components(&mut tx, &ctx.org_id, &id).await?;
    check_impacts(&input.impacts, &components).map_err(ApiError::BadRequest)?;
    if let Some(ir) = input.incident_id {
        let found: Option<(i32,)> = sqlx::query_as("SELECT public_id FROM incidents WHERE org_id = $1 AND public_id = $2").bind(&ctx.org_id).bind(ir).fetch_optional(&mut *tx).await?;
        if found.is_none() {
            return Err(ApiError::BadRequest(format!("Incident IR-{ir} not found")));
        }
    }
    let resolved = input.status == NoticeStatus::Resolved;
    let (notice,): (String,) = sqlx::query_as(
        "INSERT INTO status_page_notices (org_id, page_id, title, status, impacts, incident_id, created_by, resolved_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7::uuid, CASE WHEN $8 THEN now() END) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&title)
    .bind(input.status.as_str())
    .bind(json(&input.impacts))
    .bind(input.incident_id)
    .bind(&ctx.user_id)
    .bind(resolved)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO status_page_notice_updates (org_id, notice_id, status, message, impacts, created_by) VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid)")
        .bind(&ctx.org_id)
        .bind(&notice)
        .bind(input.status.as_str())
        .bind(&message)
        .bind(json(&input.impacts))
        .bind(&ctx.user_id)
        .execute(&mut *tx)
        .await?;
    touch(&mut tx, &ctx, &id).await?;
    tx.commit().await?;
    ctx.audit(&st, "status_page.notice_created", &notice, &format!("{title} ({})", input.status.as_str())).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// Posts an update to an open notice; `resolved` resolves it.
#[utoipa::path(post, path = "/api/v1/status-pages/{id}/notices/{notice_id}/updates", operation_id = "update_status_page_notice", params(("id" = String, Path), ("notice_id" = String, Path)),
    request_body = NoticeUpdateInput,
    responses((status = 200, body = StatusPageDetail), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn update_notice(State(st): State<AppState>, Writer(ctx): Writer, Path((id, notice_id)): Path<(String, String)>, Json(input): Json<NoticeUpdateInput>) -> ApiResult<StatusPageDetail> {
    let message = required(&input.message, "The message", MAX_MESSAGE_CHARS).map_err(ApiError::BadRequest)?;
    if !is_uuid(&notice_id) {
        return Err(notice_not_found());
    }
    let mut tx = st.postgres.begin().await?;
    let components = lock_components(&mut tx, &ctx.org_id, &id).await?;
    let current: Option<(String, Value, bool)> =
        sqlx::query_as("SELECT title, impacts, resolved_at IS NOT NULL FROM status_page_notices WHERE org_id = $1 AND page_id = $2::uuid AND id = $3::uuid FOR UPDATE")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(&notice_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((title, current_impacts, resolved)) = current else { return Err(notice_not_found()) };
    if resolved {
        return Err(ApiError::BadRequest("The notice is resolved; open a new one instead".into()));
    }
    let new_impacts = match input.impacts {
        Some(i) => {
            check_impacts(&i, &components).map_err(ApiError::BadRequest)?;
            json(&i)
        }
        None => current_impacts,
    };
    let resolves = input.status == NoticeStatus::Resolved;
    sqlx::query("INSERT INTO status_page_notice_updates (org_id, notice_id, status, message, impacts, created_by) VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid)")
        .bind(&ctx.org_id)
        .bind(&notice_id)
        .bind(input.status.as_str())
        .bind(&message)
        .bind(&new_impacts)
        .bind(&ctx.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE status_page_notices SET status = $3, impacts = $4, resolved_at = CASE WHEN $5 THEN now() END WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&notice_id)
        .bind(input.status.as_str())
        .bind(&new_impacts)
        .bind(resolves)
        .execute(&mut *tx)
        .await?;
    touch(&mut tx, &ctx, &id).await?;
    tx.commit().await?;
    let action = if resolves { "status_page.notice_resolved" } else { "status_page.notice_updated" };
    ctx.audit(&st, action, &notice_id, &format!("{title} ({})", input.status.as_str())).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// Deletes a notice posted by mistake, with its updates; it no longer counts in the uptime bars.
#[utoipa::path(delete, path = "/api/v1/status-pages/{id}/notices/{notice_id}", operation_id = "delete_status_page_notice", params(("id" = String, Path), ("notice_id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn delete_notice(State(st): State<AppState>, Writer(ctx): Writer, Path((id, notice_id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) || !is_uuid(&notice_id) {
        return Err(notice_not_found());
    }
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM status_page_notices WHERE org_id = $1 AND page_id = $2::uuid AND id = $3::uuid RETURNING title")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&notice_id)
        .fetch_optional(&st.postgres)
        .await?;
    let Some((title,)) = row else { return Err(notice_not_found()) };
    ctx.audit(&st, "status_page.notice_deleted", &notice_id, &title).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The read-only page at `/status/<slug>`, without sign-in. Public pages answer to anyone; internal
/// pages only to signed-in members of the page's org. Everyone else gets 404, as for an unknown slug.
#[utoipa::path(get, path = "/api/v1/public/status-pages/{slug}", operation_id = "get_public_status_page", params(("slug" = String, Path)),
    responses((status = 200, body = PublicStatusPage), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn get_public(State(st): State<AppState>, ctx: Option<AuthContext>, Path(slug): Path<String>) -> ApiResult<PublicStatusPage> {
    // The slug is unique across the instance; the org comes from the page it names, never from the request.
    let row: Option<(String, String, String)> = sqlx::query_as("SELECT org_id, id::text, visibility FROM status_pages WHERE slug = $1").bind(&slug).fetch_optional(&st.postgres).await?;
    let Some((org, id, vis)) = row else { return Err(page_not_found()) };
    let member = ctx.is_some_and(|c| c.org_id == org);
    if visibility(&vis) != StatusPageVisibility::Public && !member {
        return Err(page_not_found());
    }
    Ok(Json(public(load(&st, &org, &id).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ComponentStatus::*;

    fn component(id: &str, status: ComponentStatus) -> StoredComponent {
        StoredComponent { id: id.into(), name: id.to_uppercase(), group: None, status }
    }

    fn impact(id: &str, impact: ComponentStatus) -> NoticeImpact {
        NoticeImpact { component_id: id.into(), impact }
    }

    fn input(components: &[(&str, Option<&str>, Option<&str>)]) -> StatusPageInput {
        StatusPageInput {
            name: " Storefront status ".into(),
            slug: "storefront".into(),
            visibility: StatusPageVisibility::Public,
            description: "".into(),
            logo_url: "".into(),
            email_subscriptions: false,
            slack_subscriptions: false,
            components: components
                .iter()
                .map(|(name, group, id)| StatusPageComponentInput { id: id.map(Into::into), name: name.to_string(), group: group.map(Into::into), status: Operational })
                .collect(),
        }
    }

    #[test]
    fn statuses_are_ordered_from_best_to_worst() {
        assert_eq!(worst([]), Operational);
        assert_eq!(worst([DegradedPerformance, MajorOutage, PartialOutage]), MajorOutage);
        assert_eq!(worst([Operational, DegradedPerformance]), DegradedPerformance);
        assert_eq!(serde_json::to_value(DegradedPerformance).unwrap(), "degraded_performance");
    }

    #[test]
    fn open_notices_make_components_worse_but_never_better() {
        let components = [component("checkout", Operational), component("search", PartialOutage), component("cart", Operational)];
        let first = [impact("checkout", DegradedPerformance), impact("search", DegradedPerformance)];
        let second = [impact("checkout", MajorOutage), impact("gone", MajorOutage)];
        let current = current_components(&components, &[&first, &second]);
        let statuses: Vec<_> = current.iter().map(|c| (c.id.as_str(), c.status, c.current_status)).collect();
        assert_eq!(statuses, [("checkout", Operational, MajorOutage), ("search", PartialOutage, PartialOutage), ("cart", Operational, Operational)]);
    }

    #[test]
    fn slugs_are_lowercase_letters_digits_and_inner_dashes() {
        for ok in ["status", "shop-status-2", "a", "9"] {
            assert_eq!(check_slug(ok), Ok(()), "{ok}");
        }
        for bad in ["", "Status", "-status", "status-", "with space", "with_underscore", "é", &"x".repeat(MAX_SLUG_CHARS + 1)] {
            assert!(check_slug(bad).unwrap_err().contains("lowercase letters, digits and dashes"), "{bad}");
        }
    }

    #[test]
    fn required_texts_are_trimmed_and_bounded() {
        assert_eq!(required("  Checkout  ", "The name", 10), Ok("Checkout".into()));
        assert_eq!(required("   ", "The name", 10), Err("The name is required".into()));
        assert_eq!(required("abcdefghijk", "The name", 10), Err("The name is at most 10 characters".into()));
    }

    #[test]
    fn component_ids_are_random_and_prefixed() {
        let (a, b) = (new_component_id(), new_component_id());
        assert_ne!(a, b);
        assert!(a.starts_with("c_") && a.len() == 14 && a[2..].bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{a}");
    }

    #[test]
    fn pages_keep_known_component_ids_and_give_new_ones_random_ids() {
        let existing = [component("c_checkout", MajorOutage)];
        let mut page = input(&[(" Checkout ", Some(" Storefront "), Some("c_checkout")), ("Search", Some(" "), Some("c_forged")), ("Cart", None, None)]);
        let stored = check_page(&mut page, &existing).unwrap();
        assert_eq!(page.name, "Storefront status");
        assert_eq!(stored[0], StoredComponent { id: "c_checkout".into(), name: "Checkout".into(), group: Some("Storefront".into()), status: Operational });
        assert_eq!((stored[1].name.as_str(), stored[1].group.as_deref()), ("Search", None));
        assert!(stored[1].id.starts_with("c_") && stored[1].id != "c_forged");
        assert!(stored[2].id.starts_with("c_") && stored[2].id != stored[1].id);
    }

    #[test]
    fn pages_are_rejected_with_clear_errors() {
        let check = |mut page: StatusPageInput| check_page(&mut page, &[]).unwrap_err();
        let mut nameless = input(&[]);
        nameless.name = " ".into();
        assert_eq!(check(nameless), "The name is required");
        let mut bad_slug = input(&[]);
        bad_slug.slug = "Shop".into();
        assert!(check(bad_slug).contains("dashes"));
        let mut long = input(&[]);
        long.description = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert_eq!(check(long), "The description is at most 1000 characters");
        assert_eq!(check(input(&[("Checkout", None, None), (" checkout", None, None)])), "Component 'checkout' appears twice");
        assert_eq!(check(input(&[("", None, None)])), "The component name is required");
        let many: Vec<String> = (0..=MAX_COMPONENTS).map(|i| format!("c{i}")).collect();
        let many: Vec<(&str, Option<&str>, Option<&str>)> = many.iter().map(|n| (n.as_str(), None, None)).collect();
        assert_eq!(check(input(&many)), "A status page has at most 100 components");
    }

    #[test]
    fn logos_are_http_or_https_urls() {
        let check = |url: &str| {
            let mut page = input(&[]);
            page.logo_url = url.into();
            check_page(&mut page, &[]).map(|_| page.logo_url)
        };
        assert_eq!(check(""), Ok("".into()));
        assert_eq!(check(" https://cdn.example.com/logo.png "), Ok("https://cdn.example.com/logo.png".into()));
        for bad in ["javascript:alert(1)", "data:image/png;base64,AAAA", "ftp://example.com/logo.png", "logo.png"] {
            assert_eq!(check(bad), Err("The logo must be an http or https URL".into()), "{bad}");
        }
        assert_eq!(check(&format!("https://example.com/{}", "x".repeat(MAX_URL_CHARS))), Err("The logo URL is at most 2048 characters".into()));
    }

    #[test]
    fn impacts_name_components_of_the_page_once() {
        let components = [component("checkout", Operational), component("search", Operational)];
        assert_eq!(check_impacts(&[impact("checkout", MajorOutage), impact("search", DegradedPerformance)], &components), Ok(()));
        assert_eq!(check_impacts(&[impact("cart", MajorOutage)], &components), Err("Component 'cart' is not on this status page".into()));
        assert_eq!(check_impacts(&[impact("checkout", MajorOutage), impact("checkout", PartialOutage)], &components), Err("Component 'CHECKOUT' is impacted twice".into()));
        assert_eq!(check_impacts(&[impact("checkout", Operational)], &components), Err("The impact on 'CHECKOUT' must be degraded performance, partial outage or major outage".into()));
    }
}
