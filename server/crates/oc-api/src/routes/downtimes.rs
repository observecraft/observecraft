//! Downtimes: scheduled (optionally recurring) windows that silence monitor notifications. A downtime
//! targets one monitor, or every monitor carrying all of its monitor tags (none = every monitor), and
//! is limited to the groups matching its scope. Muting a monitor creates a downtime.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::identity::MemberRef;
use super::monitor_engine::{DowntimeWindow, DAY_MS};
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

#[derive(sqlx::FromRow, Clone)]
pub(crate) struct Downtime {
    pub id: i64,
    pub monitor_id: Option<i64>,
    pub monitor_name: Option<String>,
    pub monitor_tags: Vec<String>,
    pub scope: Vec<String>,
    pub message: String,
    pub start_at: i64,
    pub end_at: Option<i64>,
    pub recurrence: Option<String>,
    pub recur_until: Option<i64>,
    pub creator_id: Option<String>,
    pub creator_name: Option<String>,
    pub created_at: i64,
    pub canceled_at: Option<i64>,
}

impl Downtime {
    pub fn window(&self) -> DowntimeWindow {
        DowntimeWindow {
            start: self.start_at,
            end: self.end_at,
            period: match self.recurrence.as_deref() {
                Some("daily") => Some(DAY_MS),
                Some("weekly") => Some(7 * DAY_MS),
                _ => None,
            },
            until: self.recur_until,
            canceled: self.canceled_at,
        }
    }

    pub fn applies_to(&self, monitor_id: i64, monitor_tags: &[String]) -> bool {
        match self.monitor_id {
            Some(id) => id == monitor_id,
            None => self.monitor_tags.iter().all(|t| monitor_tags.contains(t)),
        }
    }
}

const SELECT: &str = "SELECT d.id, d.monitor_id, m.name AS monitor_name, d.monitor_tags, d.scope, d.message, \
    (extract(epoch FROM d.start_at) * 1000)::bigint AS start_at, (extract(epoch FROM d.end_at) * 1000)::bigint AS end_at, d.recurrence, \
    (extract(epoch FROM d.recur_until) * 1000)::bigint AS recur_until, d.created_by::text AS creator_id, u.name AS creator_name, \
    (extract(epoch FROM d.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM d.canceled_at) * 1000)::bigint AS canceled_at \
    FROM downtimes d LEFT JOIN monitors m ON m.id = d.monitor_id LEFT JOIN users u ON u.id = d.created_by";

/// The org's downtimes that are, or may still become, active.
pub(crate) async fn load_downtimes(st: &AppState, org: &str) -> Result<Vec<Downtime>, ApiError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE d.org_id = $1 AND d.canceled_at IS NULL")).bind(org).fetch_all(&st.postgres).await?)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DowntimeStatus {
    Active,
    Scheduled,
    Ended,
    Canceled,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Recurrence {
    Daily,
    Weekly,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DowntimeItem {
    pub id: i64,
    pub status: DowntimeStatus,
    pub monitor_id: Option<i64>,
    pub monitor_name: Option<String>,
    /// Monitors carrying all of these tags (when not targeting one monitor; empty = all monitors).
    pub monitor_tags: Vec<String>,
    /// Groups covered: `*` or `key:value` tags.
    pub scope: Vec<String>,
    pub message: String,
    pub start: i64,
    pub end: Option<i64>,
    pub recurrence: Option<Recurrence>,
    pub recur_until: Option<i64>,
    /// While active: when the current occurrence ends (absent = no end).
    pub active_until: Option<i64>,
    pub creator: Option<MemberRef>,
    pub created_at: i64,
}

impl Downtime {
    fn item(&self, now: i64) -> DowntimeItem {
        let w = self.window();
        let active = w.active_at(now);
        let status = if self.canceled_at.is_some() {
            DowntimeStatus::Canceled
        } else if active.is_some() {
            DowntimeStatus::Active
        } else if w.finished_at(now) {
            DowntimeStatus::Ended
        } else {
            DowntimeStatus::Scheduled
        };
        DowntimeItem {
            id: self.id,
            status,
            monitor_id: self.monitor_id,
            monitor_name: self.monitor_name.clone(),
            monitor_tags: self.monitor_tags.clone(),
            scope: if self.scope.is_empty() { vec!["*".into()] } else { self.scope.clone() },
            message: self.message.clone(),
            start: self.start_at,
            end: self.end_at,
            recurrence: match self.recurrence.as_deref() {
                Some("daily") => Some(Recurrence::Daily),
                Some("weekly") => Some(Recurrence::Weekly),
                _ => None,
            },
            recur_until: self.recur_until,
            active_until: active.and_then(|a| a.1),
            creator: self.creator_id.clone().map(|id| MemberRef { id, name: self.creator_name.clone().unwrap_or_default() }),
            created_at: self.created_at,
        }
    }
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DowntimeSearchRequest {
    /// Matches monitor name, message, scope and monitor tags.
    pub query: String,
    pub active: Vec<bool>,
    pub recurring: Vec<bool>,
    /// Creator user ids.
    pub creators: Vec<String>,
    /// Include canceled downtimes (last 30 days).
    pub include_canceled: bool,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct DowntimeFacetCount {
    pub value: String,
    pub label: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DowntimeFacets {
    pub active: Vec<DowntimeFacetCount>,
    pub recurring: Vec<DowntimeFacetCount>,
    pub creators: Vec<DowntimeFacetCount>,
}

#[derive(Serialize, ToSchema)]
pub struct DowntimeSearchPage {
    pub downtimes: Vec<DowntimeItem>,
    pub total: i64,
    pub facets: DowntimeFacets,
}

#[utoipa::path(post, path = "/api/v1/downtimes/search", operation_id = "search_downtimes", request_body = DowntimeSearchRequest, responses((status = 200, body = DowntimeSearchPage)), tag = "monitors")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<DowntimeSearchRequest>) -> ApiResult<DowntimeSearchPage> {
    let rows: Vec<Downtime> = sqlx::query_as(&format!(
        "{SELECT} WHERE d.org_id = $1 AND (d.canceled_at IS NULL OR ($2 AND d.canceled_at > now() - interval '30 days'))"
    ))
    .bind(&org)
    .bind(req.include_canceled)
    .fetch_all(&st.postgres)
    .await?;
    let now = now_ms();
    let words: Vec<String> = req.query.split_whitespace().map(str::to_lowercase).collect();
    let items: Vec<DowntimeItem> = rows
        .iter()
        .map(|d| d.item(now))
        .filter(|i| {
            let hay = format!("{} {} {} {}", i.monitor_name.as_deref().unwrap_or(""), i.message, i.scope.join(" "), i.monitor_tags.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect();
    let is_active = |i: &DowntimeItem| i.status == DowntimeStatus::Active;
    let pass = |i: &DowntimeItem, skip: u8| {
        (skip == 1 || req.active.is_empty() || req.active.contains(&is_active(i)))
            && (skip == 2 || req.recurring.is_empty() || req.recurring.contains(&i.recurrence.is_some()))
            && (skip == 3 || req.creators.is_empty() || i.creator.as_ref().is_some_and(|c| req.creators.contains(&c.id)))
    };
    let bool_facet = |skip: u8, f: &dyn Fn(&DowntimeItem) -> bool| {
        ["true", "false"]
            .iter()
            .map(|v| DowntimeFacetCount {
                value: v.to_string(),
                label: if *v == "true" { "True".into() } else { "False".into() },
                count: items.iter().filter(|i| pass(i, skip) && f(i).to_string() == *v).count() as i64,
            })
            .collect::<Vec<_>>()
    };
    let mut creators: Vec<DowntimeFacetCount> = Vec::new();
    for i in items.iter().filter(|i| pass(i, 3)) {
        if let Some(c) = &i.creator {
            match creators.iter_mut().find(|f| f.value == c.id) {
                Some(f) => f.count += 1,
                None => creators.push(DowntimeFacetCount { value: c.id.clone(), label: c.name.clone(), count: 1 }),
            }
        }
    }
    creators.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    let facets = DowntimeFacets { active: bool_facet(1, &is_active), recurring: bool_facet(2, &|i| i.recurrence.is_some()), creators };
    let mut matched: Vec<DowntimeItem> = items.into_iter().filter(|i| pass(i, 0)).collect();
    // Active first, then by start, newest first.
    matched.sort_by(|a, b| (b.status == DowntimeStatus::Active).cmp(&(a.status == DowntimeStatus::Active)).then_with(|| b.start.cmp(&a.start)));
    let total = matched.len() as i64;
    let downtimes = matched.into_iter().skip(req.offset.max(0) as usize).take(req.limit.clamp(1, 200) as usize).collect();
    Ok(Json(DowntimeSearchPage { downtimes, total, facets }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DowntimeInput {
    /// One monitor, or…
    pub monitor_id: Option<i64>,
    /// …every monitor carrying all of these tags (empty = all monitors).
    #[serde(default)]
    pub monitor_tags: Vec<String>,
    /// `*` or `key:value` group tags.
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub message: String,
    /// Epoch ms; absent = now.
    pub start: Option<i64>,
    /// Epoch ms; absent = until canceled (not allowed for recurring downtimes).
    pub end: Option<i64>,
    pub recurrence: Option<Recurrence>,
    pub recur_until: Option<i64>,
}

#[utoipa::path(post, path = "/api/v1/downtimes", operation_id = "create_downtime", request_body = DowntimeInput, responses((status = 200, body = DowntimeItem)), tag = "monitors")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<DowntimeInput>) -> ApiResult<DowntimeItem> {
    let now = now_ms();
    let start = input.start.unwrap_or(now);
    if let Some(end) = input.end {
        if end <= start || end <= now {
            return Err(ApiError::BadRequest("the downtime must end after it starts, in the future".into()));
        }
    }
    if let Some(r) = input.recurrence {
        let end = input.end.ok_or_else(|| ApiError::BadRequest("recurring downtimes need an end time".into()))?;
        let period = if r == Recurrence::Daily { DAY_MS } else { 7 * DAY_MS };
        if end - start >= period {
            return Err(ApiError::BadRequest("each occurrence must be shorter than the repeat period".into()));
        }
    }
    let scope: Vec<String> = input.scope.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "*").collect();
    if scope.iter().any(|s| !s.contains(':')) {
        return Err(ApiError::BadRequest("scope entries are key:value tags, e.g. host:web-1".into()));
    }
    if let Some(id) = input.monitor_id {
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM monitors WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(id).fetch_optional(&st.postgres).await?;
        exists.ok_or_else(|| ApiError::NotFound(format!("monitor {id} not found")))?;
    }
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO downtimes (org_id, monitor_id, monitor_tags, scope, message, start_at, end_at, recurrence, recur_until, created_by) \
         VALUES ($1, $2, $3, $4, $5, to_timestamp($6), to_timestamp($7), $8, to_timestamp($9), $10::uuid) RETURNING id",
    )
    .bind(&ctx.org_id)
    .bind(input.monitor_id)
    .bind(if input.monitor_id.is_some() { Vec::new() } else { input.monitor_tags.clone() })
    .bind(&scope)
    .bind(input.message.trim())
    .bind(start as f64 / 1000.0)
    .bind(input.end.map(|e| e as f64 / 1000.0))
    .bind(input.recurrence.map(|r| if r == Recurrence::Daily { "daily" } else { "weekly" }))
    .bind(input.recur_until.map(|e| e as f64 / 1000.0))
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "downtime.create", &format!("downtime:{id}"), &input.message).await;
    let d: Downtime = sqlx::query_as(&format!("{SELECT} WHERE d.id = $1")).bind(id).fetch_one(&st.postgres).await?;
    Ok(Json(d.item(now_ms())))
}

#[utoipa::path(delete, path = "/api/v1/downtimes/{id}", operation_id = "cancel_downtime", params(("id" = i64, Path)), responses((status = 204)), tag = "monitors")]
pub async fn cancel(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let r = sqlx::query("UPDATE downtimes SET canceled_at = now() WHERE org_id = $1 AND id = $2 AND canceled_at IS NULL").bind(&ctx.org_id).bind(id).execute(&st.postgres).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("downtime {id} not found")));
    }
    ctx.audit(&st, "downtime.cancel", &format!("downtime:{id}"), "").await;
    Ok(StatusCode::NO_CONTENT)
}
