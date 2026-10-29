//! Monitors: definitions, the background evaluator, triggered groups, history, backtest previews,
//! bulk actions and the quality report. Evaluation logic lives in [`super::monitor_engine`];
//! downtimes (muting) in [`super::downtimes`].
//!
//! Lists are filtered and faceted in memory: an org has at most a few thousand monitors, and facet
//! counts depend on derived values (muted, recipients) that SQL can't see.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use futures::StreamExt;
use oc_query::parse_metric_query;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::downtimes::{load_downtimes, Downtime};
use super::identity::MemberRef;
use super::logs::{log_counts, parse_log_monitor, TimeRange};
use super::metrics::{evaluate_query, Group};
use super::monitor_formula::{combine, parse_formula, Expr};
use super::monitor_engine::{
    backtest, group_key, parse_group_key, recipients, render_message, worst, MessageContext, MonitorOptions, MonitorStatus, DAY_MS,
};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::{AppState, OrgId};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MonitorType {
    /// Any metric query (host and OTel metrics).
    Metric,
    /// `trace.*` metrics computed from spans.
    Apm,
    /// Count of logs matching a search: `logs("service:checkout status:error") by {service}`.
    Log,
    /// The alert state of a synthetic test, kept by the test runner (`routes::synthetics`); edited
    /// with its test, never evaluated here.
    Synthetics,
}

impl MonitorType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MonitorType::Metric => "metric",
            MonitorType::Apm => "apm",
            MonitorType::Log => "log",
            MonitorType::Synthetics => "synthetics",
        }
    }
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "apm" => MonitorType::Apm,
            "log" => MonitorType::Log,
            "synthetics" => MonitorType::Synthetics,
            _ => MonitorType::Metric,
        }
    }
}

/// The series a monitor evaluates over `range`: its metric query's groups, or for log monitors the
/// matching logs per interval (per group), counted within `access` (the previewing user's, or
/// every log of the org when the engine evaluates).
async fn monitor_series(st: &AppState, org: &str, access: &LogAccess, kind: MonitorType, query: &str, range: TimeRange, points: u32) -> Result<Vec<Group>, ApiError> {
    if kind == MonitorType::Synthetics {
        return Err(ApiError::BadRequest("synthetic test monitors have no query to evaluate".into()));
    }
    if kind == MonitorType::Log {
        let (search, by) = parse_log_monitor(query).map_err(ApiError::BadRequest)?;
        let step = ((range.to - range.from) / points.max(1) as i64).max(60_000);
        return log_counts(st, org, access, &search, &by, range, step).await;
    }
    let query = super::derived_metrics::resolve_text(st, org, query).await?;
    let f = parse_formula(&query).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
    let mut results = Vec::with_capacity(f.queries.len());
    for q in &f.queries {
        let q = parse_metric_query(q).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
        results.push(evaluate_query(st, org, &q, range, points).await?);
    }
    // One plain query: its groups as they are.
    if f.expr == Expr::Query(0) {
        return Ok(results.remove(0));
    }
    Ok(combine(&f.expr, &results))
}

// ================================================================ loading

#[derive(sqlx::FromRow, Clone)]
pub(crate) struct Row {
    pub id: i64,
    pub org_id: String,
    pub name: String,
    pub kind: String,
    pub query: String,
    pub definition: Value,
    pub status: String,
    pub message: String,
    pub priority: Option<i16>,
    pub tags: Vec<String>,
    pub teams: Vec<String>,
    pub draft: bool,
    pub creator_id: Option<String>,
    pub creator_name: Option<String>,
    pub created_at: i64,
    pub modified_at: i64,
    pub state_changed_at: i64,
    pub evaluated_at: Option<i64>,
}

const SELECT: &str = "SELECT m.id, m.org_id, m.name, m.type AS kind, m.query, m.definition, m.status, m.message, m.priority, m.tags, m.teams, m.draft, \
    m.created_by::text AS creator_id, u.name AS creator_name, \
    (extract(epoch FROM m.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM m.modified_at) * 1000)::bigint AS modified_at, \
    (extract(epoch FROM m.state_changed_at) * 1000)::bigint AS state_changed_at, (extract(epoch FROM m.evaluated_at) * 1000)::bigint AS evaluated_at \
    FROM monitors m LEFT JOIN users u ON u.id = m.created_by";

#[derive(sqlx::FromRow, Clone)]
pub(crate) struct GroupRow {
    pub monitor_id: i64,
    pub group_key: String,
    pub status: String,
    pub value: Option<f64>,
    pub last_data_at: Option<i64>,
    pub state_changed_at: i64,
}

const GROUP_SELECT: &str = "SELECT g.monitor_id, g.group_key, g.status, g.value, (extract(epoch FROM g.last_data_at) * 1000)::bigint AS last_data_at, \
    (extract(epoch FROM g.state_changed_at) * 1000)::bigint AS state_changed_at FROM monitor_groups g";

/// A monitor with everything derived from its definition, groups and the org's downtimes.
pub(crate) struct Loaded {
    pub row: Row,
    pub opts: Option<MonitorOptions>,
    pub status: MonitorStatus,
    pub recipients: Vec<String>,
    pub groups: Vec<GroupRow>,
    /// `Some(None)` = muted with no end, `Some(Some(t))` = muted until t.
    pub muted: Option<Option<i64>>,
}

impl Loaded {
    fn new(row: Row, groups: Vec<GroupRow>, downtimes: &[Downtime], now: i64) -> Self {
        let opts = serde_json::from_value(row.definition.clone()).ok();
        let recipients = recipients(&row.message);
        let muted = muted_until(&row, downtimes, now);
        Loaded { status: MonitorStatus::parse(&row.status), opts, recipients, groups, muted, row }
    }

    fn item(&self) -> MonitorItem {
        let r = &self.row;
        let mut counts = GroupCounts::default();
        for g in &self.groups {
            match MonitorStatus::parse(&g.status) {
                MonitorStatus::Alert => counts.alert += 1,
                MonitorStatus::Warn => counts.warn += 1,
                MonitorStatus::NoData => counts.nodata += 1,
                MonitorStatus::Ok => counts.ok += 1,
            }
        }
        MonitorItem {
            id: r.id,
            name: r.name.clone(),
            r#type: MonitorType::parse(&r.kind),
            status: self.status,
            priority: r.priority,
            tags: r.tags.clone(),
            teams: r.teams.clone(),
            draft: r.draft,
            creator: r.creator_id.clone().map(|id| MemberRef { id, name: r.creator_name.clone().unwrap_or_default() }),
            recipients: self.recipients.clone(),
            muted: self.muted.is_some(),
            muted_until: self.muted.flatten(),
            query: self.opts.as_ref().map(|o| o.full_query(&r.query)).unwrap_or_else(|| r.query.clone()),
            group_counts: counts,
            state_changed_at: r.state_changed_at,
            created_at: r.created_at,
            modified_at: r.modified_at,
        }
    }

    fn group_muted(&self, key: &str, downtimes: &[Downtime], now: i64) -> Option<Option<i64>> {
        let tags = parse_group_key(key);
        active_for(&self.row, downtimes, now, |d| super::monitor_engine::scope_covers(&d.scope, &tags))
    }
}

/// Latest end among the active downtimes that apply to the monitor and pass `covers`.
fn active_for(row: &Row, downtimes: &[Downtime], now: i64, covers: impl Fn(&Downtime) -> bool) -> Option<Option<i64>> {
    let mut out: Option<Option<i64>> = None;
    for d in downtimes.iter().filter(|d| d.applies_to(row.id, &row.tags) && covers(d)) {
        if let Some((_, end)) = d.window().active_at(now) {
            out = Some(match (out, end) {
                (Some(None), _) | (_, None) => None,
                (Some(Some(a)), Some(b)) => Some(a.max(b)),
                (None, Some(b)) => Some(b),
            });
        }
    }
    out
}

fn muted_until(row: &Row, downtimes: &[Downtime], now: i64) -> Option<Option<i64>> {
    active_for(row, downtimes, now, |d| d.scope.iter().all(|s| s == "*"))
}

pub(crate) async fn load_all(st: &AppState, org: &str) -> Result<(Vec<Loaded>, Vec<Downtime>), ApiError> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE m.org_id = $1")).bind(org).fetch_all(&st.postgres).await?;
    let groups: Vec<GroupRow> = sqlx::query_as(&format!("{GROUP_SELECT} JOIN monitors m ON m.id = g.monitor_id WHERE m.org_id = $1 ORDER BY g.group_key"))
        .bind(org)
        .fetch_all(&st.postgres)
        .await?;
    let downtimes = load_downtimes(st, org).await?;
    let mut by_monitor: HashMap<i64, Vec<GroupRow>> = HashMap::new();
    for g in groups {
        by_monitor.entry(g.monitor_id).or_default().push(g);
    }
    let now = now_ms();
    let loaded = rows
        .into_iter()
        .map(|r| {
            let groups = by_monitor.remove(&r.id).unwrap_or_default();
            Loaded::new(r, groups, &downtimes, now)
        })
        .collect();
    Ok((loaded, downtimes))
}

pub(crate) async fn load_one(st: &AppState, org: &str, id: i64) -> Result<(Loaded, Vec<Downtime>), ApiError> {
    let row: Row = sqlx::query_as(&format!("{SELECT} WHERE m.org_id = $1 AND m.id = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("monitor {id} not found")))?;
    let groups: Vec<GroupRow> = sqlx::query_as(&format!("{GROUP_SELECT} WHERE g.monitor_id = $1 ORDER BY g.group_key")).bind(id).fetch_all(&st.postgres).await?;
    let downtimes = load_downtimes(st, org).await?;
    Ok((Loaded::new(row, groups, &downtimes, now_ms()), downtimes))
}

// ================================================================ list

#[derive(Serialize, ToSchema, Default, Clone)]
pub struct GroupCounts {
    pub alert: i64,
    pub warn: i64,
    pub nodata: i64,
    pub ok: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorItem {
    pub id: i64,
    pub name: String,
    pub r#type: MonitorType,
    pub status: MonitorStatus,
    /// 1 (critical) … 5; none when unset.
    pub priority: Option<i16>,
    pub tags: Vec<String>,
    pub teams: Vec<String>,
    pub draft: bool,
    pub creator: Option<MemberRef>,
    /// `@handles` found in the message.
    pub recipients: Vec<String>,
    pub muted: bool,
    /// End of the muting downtime; absent while muted = muted indefinitely.
    pub muted_until: Option<i64>,
    /// Full query, e.g. `avg(last_5m):avg:system.load.1{*} by {host} > 2`.
    pub query: String,
    pub group_counts: GroupCounts,
    pub state_changed_at: i64,
    pub created_at: i64,
    pub modified_at: i64,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct FacetCount {
    pub value: String,
    pub count: i64,
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MonitorSearchRequest {
    /// Words match name, query and tags; `key:value` words match tags (`service:billing`).
    pub query: String,
    pub statuses: Vec<MonitorStatus>,
    pub muted: Vec<bool>,
    /// 0 = no priority.
    pub priorities: Vec<i16>,
    pub types: Vec<MonitorType>,
    pub drafts: Vec<bool>,
    pub recipients: Vec<String>,
    pub creators: Vec<String>,
    pub teams: Vec<String>,
    /// status (default) | name | priority | modified
    pub sort: Option<String>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema, Default)]
pub struct MonitorFacets {
    pub status: Vec<FacetCount>,
    pub muted: Vec<FacetCount>,
    pub priority: Vec<FacetCount>,
    pub r#type: Vec<FacetCount>,
    pub draft: Vec<FacetCount>,
    pub recipients: Vec<FacetCount>,
    pub creators: Vec<FacetCount>,
    pub teams: Vec<FacetCount>,
}

#[derive(Serialize, ToSchema)]
pub struct MonitorSearchPage {
    pub monitors: Vec<MonitorItem>,
    pub total: i64,
    pub facets: MonitorFacets,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Facet {
    Status,
    Muted,
    Priority,
    Type,
    Draft,
    Recipient,
    Creator,
    Team,
}

fn text_matches(query: &str, name: &str, full_query: &str, tags: &[String]) -> bool {
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        if w.contains(':') && !w.starts_with(':') {
            return tags.iter().any(|t| t.to_lowercase() == w);
        }
        name.to_lowercase().contains(&w) || full_query.to_lowercase().contains(&w) || tags.iter().any(|t| t.to_lowercase().contains(&w))
    })
}

fn passes(m: &Loaded, item: &MonitorItem, req: &MonitorSearchRequest, skip: Option<Facet>) -> bool {
    let on = |f: Facet| skip != Some(f);
    (!on(Facet::Status) || req.statuses.is_empty() || req.statuses.contains(&m.status))
        && (!on(Facet::Muted) || req.muted.is_empty() || req.muted.contains(&item.muted))
        && (!on(Facet::Priority) || req.priorities.is_empty() || req.priorities.contains(&item.priority.unwrap_or(0)))
        && (!on(Facet::Type) || req.types.is_empty() || req.types.contains(&item.r#type))
        && (!on(Facet::Draft) || req.drafts.is_empty() || req.drafts.contains(&item.draft))
        && (!on(Facet::Recipient) || req.recipients.is_empty() || item.recipients.iter().any(|r| req.recipients.contains(r)))
        && (!on(Facet::Creator) || req.creators.is_empty() || item.creator.as_ref().is_some_and(|c| req.creators.contains(&c.id)))
        && (!on(Facet::Team) || req.teams.is_empty() || item.teams.iter().any(|t| req.teams.contains(t)))
}

fn counts(values: impl Iterator<Item = String>) -> Vec<FacetCount> {
    let mut map: BTreeMap<String, i64> = BTreeMap::new();
    for v in values {
        *map.entry(v).or_default() += 1;
    }
    let mut out: Vec<FacetCount> = map.into_iter().map(|(value, count)| FacetCount { value, count }).collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    out
}

#[utoipa::path(post, path = "/api/v1/monitors/search", operation_id = "search_monitors", request_body = MonitorSearchRequest, responses((status = 200, body = MonitorSearchPage)), tag = "monitors")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<MonitorSearchRequest>) -> ApiResult<MonitorSearchPage> {
    let (loaded, _) = load_all(&st, &org).await?;
    let items: Vec<(&Loaded, MonitorItem)> = loaded.iter().map(|m| (m, m.item())).filter(|(m, i)| text_matches(&req.query, &m.row.name, &i.query, &i.tags)).collect();

    let facet = |f: Facet, key: &dyn Fn(&MonitorItem) -> Vec<String>| counts(items.iter().filter(|(m, i)| passes(m, i, &req, Some(f))).flat_map(|(_, i)| key(i)));
    let mut creator_names: HashMap<String, String> = HashMap::new();
    for (_, i) in &items {
        if let Some(c) = &i.creator {
            creator_names.insert(c.id.clone(), c.name.clone());
        }
    }
    let mut facets = MonitorFacets {
        status: facet(Facet::Status, &|i| vec![i.status.as_str().to_string()]),
        muted: facet(Facet::Muted, &|i| vec![i.muted.to_string()]),
        priority: facet(Facet::Priority, &|i| vec![i.priority.unwrap_or(0).to_string()]),
        r#type: facet(Facet::Type, &|i| vec![i.r#type.as_str().to_string()]),
        draft: facet(Facet::Draft, &|i| vec![i.draft.to_string()]),
        recipients: facet(Facet::Recipient, &|i| i.recipients.clone()),
        creators: facet(Facet::Creator, &|i| i.creator.iter().map(|c| c.id.clone()).collect()),
        teams: facet(Facet::Team, &|i| i.teams.clone()),
    };
    // Creator facet values are ids; show names.
    for c in &mut facets.creators {
        if let Some(n) = creator_names.get(&c.value) {
            c.value = format!("{}|{}", c.value, n);
        }
    }

    let mut matched: Vec<MonitorItem> = items.into_iter().filter(|(m, i)| passes(m, i, &req, None)).map(|(_, i)| i).collect();
    match req.sort.as_deref() {
        Some("name") => matched.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        Some("priority") => matched.sort_by_key(|m| (m.priority.unwrap_or(9), m.name.to_lowercase())),
        Some("modified") => matched.sort_by(|a, b| b.modified_at.cmp(&a.modified_at)),
        _ => matched.sort_by(|a, b| b.status.severity().cmp(&a.status.severity()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))),
    }
    let total = matched.len() as i64;
    let monitors = matched.into_iter().skip(req.offset.max(0) as usize).take(req.limit.clamp(1, 200) as usize).collect();
    Ok(Json(MonitorSearchPage { monitors, total, facets }))
}

// ================================================================ triggered

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TriggeredSearchRequest {
    pub query: String,
    /// alert | warn | nodata (OK groups are never listed).
    pub statuses: Vec<MonitorStatus>,
    pub muted: Vec<bool>,
    pub priorities: Vec<i16>,
    pub drafts: Vec<bool>,
    /// Only groups that changed state within this many minutes.
    pub triggered_within: Option<i64>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggeredGroup {
    pub monitor_id: i64,
    pub name: String,
    pub priority: Option<i16>,
    pub draft: bool,
    pub status: MonitorStatus,
    /// `*` or `host:a,service:b`.
    pub group: String,
    pub value: Option<f64>,
    pub triggered_at: i64,
    pub muted: bool,
    pub muted_until: Option<i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggeredFacets {
    pub status: Vec<FacetCount>,
    pub muted: Vec<FacetCount>,
    pub priority: Vec<FacetCount>,
    pub draft: Vec<FacetCount>,
    /// Groups triggered within 30, 60, 720 and 1440 minutes.
    pub triggered_within: Vec<FacetCount>,
}

#[derive(Serialize, ToSchema)]
pub struct TriggeredPage {
    pub groups: Vec<TriggeredGroup>,
    pub total: i64,
    pub facets: TriggeredFacets,
}

#[utoipa::path(post, path = "/api/v1/monitors/triggered", request_body = TriggeredSearchRequest, responses((status = 200, body = TriggeredPage)), tag = "monitors")]
pub async fn triggered(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<TriggeredSearchRequest>) -> ApiResult<TriggeredPage> {
    let (loaded, downtimes) = load_all(&st, &org).await?;
    let now = now_ms();
    let mut rows: Vec<TriggeredGroup> = Vec::new();
    for m in &loaded {
        let full = m.opts.as_ref().map(|o| o.full_query(&m.row.query)).unwrap_or_default();
        if !text_matches(&req.query, &m.row.name, &full, &m.row.tags) {
            continue;
        }
        for g in &m.groups {
            let status = MonitorStatus::parse(&g.status);
            if status == MonitorStatus::Ok {
                continue;
            }
            let muted = m.group_muted(&g.group_key, &downtimes, now);
            rows.push(TriggeredGroup {
                monitor_id: m.row.id,
                name: m.row.name.clone(),
                priority: m.row.priority,
                draft: m.row.draft,
                status,
                group: g.group_key.clone(),
                value: g.value,
                triggered_at: g.state_changed_at,
                muted: muted.is_some(),
                muted_until: muted.flatten(),
            });
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum F {
        Status,
        Muted,
        Priority,
        Draft,
        Within,
    }
    let pass = |r: &TriggeredGroup, skip: Option<F>| {
        let on = |f: F| skip != Some(f);
        (!on(F::Status) || req.statuses.is_empty() || req.statuses.contains(&r.status))
            && (!on(F::Muted) || req.muted.is_empty() || req.muted.contains(&r.muted))
            && (!on(F::Priority) || req.priorities.is_empty() || req.priorities.contains(&r.priority.unwrap_or(0)))
            && (!on(F::Draft) || req.drafts.is_empty() || req.drafts.contains(&r.draft))
            && (!on(F::Within) || req.triggered_within.map_or(true, |m| now - r.triggered_at <= m * 60_000))
    };
    let facet = |f: F, key: &dyn Fn(&TriggeredGroup) -> String| counts(rows.iter().filter(|r| pass(r, Some(f))).map(key));
    let within_base: Vec<&TriggeredGroup> = rows.iter().filter(|r| pass(r, Some(F::Within))).collect();
    let facets = TriggeredFacets {
        status: facet(F::Status, &|r| r.status.as_str().to_string()),
        muted: facet(F::Muted, &|r| r.muted.to_string()),
        priority: facet(F::Priority, &|r| r.priority.unwrap_or(0).to_string()),
        draft: facet(F::Draft, &|r| r.draft.to_string()),
        triggered_within: [30, 60, 720, 1440]
            .iter()
            .map(|m| FacetCount { value: m.to_string(), count: within_base.iter().filter(|r| now - r.triggered_at <= m * 60_000).count() as i64 })
            .collect(),
    };
    let mut matched: Vec<TriggeredGroup> = rows.into_iter().filter(|r| pass(r, None)).collect();
    matched.sort_by(|a, b| b.triggered_at.cmp(&a.triggered_at).then_with(|| b.status.severity().cmp(&a.status.severity())));
    let total = matched.len() as i64;
    let groups = matched.into_iter().skip(req.offset.max(0) as usize).take(req.limit.clamp(1, 200) as usize).collect();
    Ok(Json(TriggeredPage { groups, total, facets }))
}

// ================================================================ detail

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorGroupState {
    pub group: String,
    pub status: MonitorStatus,
    pub value: Option<f64>,
    pub last_data_at: Option<i64>,
    pub state_changed_at: i64,
    pub muted: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDetail {
    #[serde(flatten)]
    pub summary: MonitorItem,
    /// The metric query alone (`avg:system.load.1{*} by {host}`).
    pub metric_query: String,
    pub options: Option<MonitorOptions>,
    pub message: String,
    pub groups: Vec<MonitorGroupState>,
    pub evaluated_at: Option<i64>,
    /// Synthetics monitors: the test whose alert state this is.
    pub synthetic_test_id: Option<String>,
}

fn detail(m: &Loaded, downtimes: &[Downtime]) -> MonitorDetail {
    let now = now_ms();
    MonitorDetail {
        summary: m.item(),
        metric_query: m.row.query.clone(),
        options: m.opts.clone(),
        message: m.row.message.clone(),
        groups: m
            .groups
            .iter()
            .map(|g| MonitorGroupState {
                group: g.group_key.clone(),
                status: MonitorStatus::parse(&g.status),
                value: g.value,
                last_data_at: g.last_data_at,
                state_changed_at: g.state_changed_at,
                muted: m.group_muted(&g.group_key, downtimes, now).is_some(),
            })
            .collect(),
        evaluated_at: m.row.evaluated_at,
        synthetic_test_id: m.row.definition.get("syntheticTestId").and_then(Value::as_str).map(str::to_string),
    }
}

#[utoipa::path(get, path = "/api/v1/monitors/{id}", operation_id = "get_monitor", params(("id" = i64, Path)), responses((status = 200, body = MonitorDetail)), tag = "monitors")]
pub async fn get_one(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i64>) -> ApiResult<MonitorDetail> {
    let (m, downtimes) = load_one(&st, &org, id).await?;
    Ok(Json(detail(&m, &downtimes)))
}

// ================================================================ create / update / delete

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInput {
    pub name: String,
    pub r#type: MonitorType,
    /// Metric query: `avg:system.load.1{env:prod} by {host}`.
    pub query: String,
    pub options: MonitorOptions,
    /// Notification message; `@handles` are its recipients.
    #[serde(default)]
    pub message: String,
    pub priority: Option<i16>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub teams: Vec<String>,
    /// Drafts are evaluated but never notify.
    #[serde(default)]
    pub draft: bool,
}

pub(crate) const SYNTHETICS_MANAGED: &str = "synthetic test monitors are created and edited with their test (Digital Experience → Synthetic Tests)";

pub(crate) fn validate(input: &mut MonitorInput) -> Result<(), ApiError> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() || input.name.len() > 500 {
        return Err(ApiError::BadRequest("the monitor name must be 1–500 characters".into()));
    }
    if input.r#type == MonitorType::Synthetics {
        return Err(ApiError::BadRequest(SYNTHETICS_MANAGED.into()));
    }
    if input.r#type == MonitorType::Log {
        parse_log_monitor(&input.query).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
    } else {
        let f = parse_formula(&input.query).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
        let apm_only = f.queries.iter().all(|q| parse_metric_query(q).is_ok_and(|q| q.metric.starts_with("trace.")));
        if input.r#type == MonitorType::Apm && !apm_only {
            return Err(ApiError::BadRequest("APM monitors use trace.* metrics".into()));
        }
    }
    input.options.validate().map_err(ApiError::BadRequest)?;
    if input.priority.is_some_and(|p| !(1..=5).contains(&p)) {
        return Err(ApiError::BadRequest("priority must be P1–P5".into()));
    }
    if input.message.len() > 10_000 {
        return Err(ApiError::BadRequest("the message is limited to 10,000 characters".into()));
    }
    input.tags = input.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    input.tags.dedup();
    if input.tags.len() > 50 || input.tags.iter().any(|t| t.len() > 200 || t.contains(',')) {
        return Err(ApiError::BadRequest("up to 50 tags of at most 200 characters, without commas".into()));
    }
    Ok(())
}

#[utoipa::path(post, path = "/api/v1/monitors", operation_id = "create_monitor", request_body = MonitorInput, responses((status = 200, body = MonitorDetail)), tag = "monitors")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<MonitorInput>) -> ApiResult<MonitorDetail> {
    validate(&mut input)?;
    let id = store_new(&st, &ctx, &input, None).await?;
    let (m, downtimes) = load_one(&st, &ctx.org_id, id).await?;
    Ok(Json(detail(&m, &downtimes)))
}

/// Inserts a validated monitor and evaluates it once. `datadog_document`: the Datadog document it
/// was written from ([`super::datadog_api::monitors`]).
pub(crate) async fn store_new(st: &AppState, ctx: &AuthContext, input: &MonitorInput, datadog_document: Option<&Value>) -> Result<i64, ApiError> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO monitors (org_id, name, type, query, definition, status, message, priority, tags, teams, draft, created_by, datadog_document) \
         VALUES ($1, $2, $3, $4, $5, 'nodata', $6, $7, $8, $9, $10, $11::uuid, $12) RETURNING id",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(input.r#type.as_str())
    .bind(input.query.trim())
    .bind(serde_json::to_value(&input.options).map_err(anyhow::Error::from)?)
    .bind(&input.message)
    .bind(input.priority)
    .bind(&input.tags)
    .bind(&input.teams)
    .bind(input.draft)
    .bind(&ctx.user_id)
    .bind(datadog_document)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(st, "monitor.create", &format!("monitor:{id}"), &input.name).await;
    evaluate_one(st, id).await;
    Ok(id)
}

#[utoipa::path(put, path = "/api/v1/monitors/{id}", operation_id = "update_monitor", params(("id" = i64, Path)), request_body = MonitorInput, responses((status = 200, body = MonitorDetail)), tag = "monitors")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i64>, Json(mut input): Json<MonitorInput>) -> ApiResult<MonitorDetail> {
    validate(&mut input)?;
    store_update(&st, &ctx, id, &input, None).await?;
    let (m, downtimes) = load_one(&st, &ctx.org_id, id).await?;
    Ok(Json(detail(&m, &downtimes)))
}

/// Updates a monitor with a validated definition; a new query or thresholds re-evaluate its groups
/// from scratch. `datadog_document`: the Datadog document it was written from, or None to keep the
/// stored one.
pub(crate) async fn store_update(st: &AppState, ctx: &AuthContext, id: i64, input: &MonitorInput, datadog_document: Option<&Value>) -> Result<(), ApiError> {
    let (before, _) = load_one(st, &ctx.org_id, id).await?;
    if MonitorType::parse(&before.row.kind) == MonitorType::Synthetics {
        return Err(ApiError::BadRequest(SYNTHETICS_MANAGED.into()));
    }
    let definition = serde_json::to_value(&input.options).map_err(anyhow::Error::from)?;
    let reset = before.row.query != input.query.trim() || before.row.definition != definition;
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "UPDATE monitors SET name = $3, type = $4, query = $5, definition = $6, message = $7, priority = $8, tags = $9, teams = $10, draft = $11, modified_at = now(), \
         datadog_document = COALESCE($12, datadog_document) WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(&input.name)
    .bind(input.r#type.as_str())
    .bind(input.query.trim())
    .bind(&definition)
    .bind(&input.message)
    .bind(input.priority)
    .bind(&input.tags)
    .bind(&input.teams)
    .bind(input.draft)
    .bind(datadog_document)
    .execute(&mut *tx)
    .await?;
    if reset {
        // A new query or thresholds: groups are re-evaluated from scratch.
        sqlx::query("DELETE FROM monitor_groups WHERE monitor_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE monitors SET status = 'nodata', state_changed_at = now() WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(st, "monitor.update", &format!("monitor:{id}"), &input.name).await;
    if reset {
        evaluate_one(st, id).await;
    }
    Ok(())
}

#[utoipa::path(post, path = "/api/v1/monitors/{id}/clone", operation_id = "clone_monitor", params(("id" = i64, Path)), responses((status = 200, body = MonitorDetail)), tag = "monitors")]
pub async fn clone(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<i64>) -> ApiResult<MonitorDetail> {
    let new: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO monitors (org_id, name, type, query, definition, status, message, priority, tags, teams, draft, created_by) \
         SELECT org_id, '[Clone] ' || name, type, query, definition, 'nodata', message, priority, tags, teams, draft, $3::uuid FROM monitors WHERE org_id = $1 AND id = $2 AND type <> 'synthetics' RETURNING id",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(&ctx.user_id)
    .fetch_optional(&st.postgres)
    .await?;
    let (new,) = new.ok_or_else(|| ApiError::NotFound(format!("monitor {id} not found")))?;
    ctx.audit(&st, "monitor.create", &format!("monitor:{new}"), &format!("clone of monitor {id}")).await;
    evaluate_one(&st, new).await;
    let (m, downtimes) = load_one(&st, &ctx.org_id, new).await?;
    Ok(Json(detail(&m, &downtimes)))
}

#[derive(Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BulkAction {
    Mute,
    Unmute,
    Resolve,
    Delete,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorBulkRequest {
    pub ids: Vec<i64>,
    pub action: BulkAction,
    /// Mute only: minutes until the mute ends; absent = until unmuted.
    pub mute_minutes: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct MonitorBulkResult {
    pub affected: i64,
}

/// Mute (creates a downtime per monitor), unmute (cancels the monitor's downtimes), resolve (sets
/// every triggered group back to OK until the next evaluation says otherwise) or delete.
#[utoipa::path(post, path = "/api/v1/monitors/bulk", request_body = MonitorBulkRequest, responses((status = 200, body = MonitorBulkResult)), tag = "monitors")]
pub async fn bulk(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<MonitorBulkRequest>) -> ApiResult<MonitorBulkResult> {
    if req.ids.is_empty() || req.ids.len() > 1000 {
        return Err(ApiError::BadRequest("select 1–1000 monitors".into()));
    }
    // A synthetic test's monitor goes with its test: deleting it here is skipped.
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM monitors WHERE org_id = $1 AND id = ANY($2) AND ($3 OR type <> 'synthetics')")
        .bind(&ctx.org_id)
        .bind(&req.ids)
        .bind(req.action != BulkAction::Delete)
        .fetch_all(&st.postgres)
        .await?;
    let mut tx = st.postgres.begin().await?;
    match req.action {
        BulkAction::Mute => {
            if req.mute_minutes.is_some_and(|m| !(1..=525_600).contains(&m)) {
                return Err(ApiError::BadRequest("mute for 1 minute to 1 year".into()));
            }
            for id in &ids {
                sqlx::query(
                    "INSERT INTO downtimes (org_id, monitor_id, scope, message, end_at, created_by) \
                     VALUES ($1, $2, '{*}', 'Muted from the monitor', now() + make_interval(mins => $3::int), $4::uuid)",
                )
                .bind(&ctx.org_id)
                .bind(id)
                .bind(req.mute_minutes.map(|m| m as i32))
                .bind(&ctx.user_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        BulkAction::Unmute => {
            sqlx::query("UPDATE downtimes SET canceled_at = now() WHERE org_id = $1 AND monitor_id = ANY($2) AND canceled_at IS NULL AND start_at <= now()")
                .bind(&ctx.org_id)
                .bind(&ids)
                .execute(&mut *tx)
                .await?;
        }
        BulkAction::Resolve => {
            sqlx::query(
                "INSERT INTO monitor_events (monitor_id, group_key, from_status, to_status, muted, message) \
                 SELECT monitor_id, group_key, status, 'ok', true, 'Resolved manually by ' || $2 FROM monitor_groups WHERE monitor_id = ANY($1) AND status <> 'ok'",
            )
            .bind(&ids)
            .bind(&ctx.name)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE monitor_groups SET status = 'ok', state_changed_at = now() WHERE monitor_id = ANY($1) AND status <> 'ok'").bind(&ids).execute(&mut *tx).await?;
            sqlx::query("UPDATE monitors SET status = 'ok', state_changed_at = now() WHERE id = ANY($1) AND status <> 'ok'").bind(&ids).execute(&mut *tx).await?;
        }
        BulkAction::Delete => {
            sqlx::query("DELETE FROM monitors WHERE org_id = $1 AND id = ANY($2) AND type <> 'synthetics'").bind(&ctx.org_id).bind(&ids).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    let action = format!("monitor.{}", format!("{:?}", req.action).to_lowercase());
    for id in &ids {
        ctx.audit(&st, &action, &format!("monitor:{id}"), "").await;
    }
    Ok(Json(MonitorBulkResult { affected: ids.len() as i64 }))
}

// ================================================================ history

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MonitorEvent {
    pub id: i64,
    pub group: String,
    pub from_status: String,
    pub to_status: String,
    pub value: Option<f64>,
    /// Notifications suppressed (downtime, draft or manual resolve).
    pub muted: bool,
    pub recipients: Vec<String>,
    /// Rendered notification message.
    pub message: String,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusSegment {
    pub group: String,
    pub status: MonitorStatus,
    pub from: i64,
    pub to: i64,
}

#[derive(Serialize, ToSchema)]
pub struct MonitorHistory {
    /// Newest first.
    pub events: Vec<MonitorEvent>,
    /// Each group's status over the range (for the status bars).
    pub segments: Vec<StatusSegment>,
}

#[derive(Deserialize, ToSchema)]
pub struct MonitorHistoryRequest {
    pub range: TimeRange,
}

#[utoipa::path(post, path = "/api/v1/monitors/{id}/history", params(("id" = i64, Path)), request_body = MonitorHistoryRequest, responses((status = 200, body = MonitorHistory)), tag = "monitors")]
pub async fn history(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i64>, Json(req): Json<MonitorHistoryRequest>) -> ApiResult<MonitorHistory> {
    req.range.validate()?;
    let (m, _) = load_one(&st, &org, id).await?;
    let events: Vec<MonitorEvent> = sqlx::query_as(
        "SELECT id, group_key AS group, from_status, to_status, value, muted, recipients, message, (extract(epoch FROM created_at) * 1000)::bigint AS created_at \
         FROM monitor_events WHERE monitor_id = $1 AND created_at >= to_timestamp($2) AND created_at <= to_timestamp($3) ORDER BY created_at",
    )
    .bind(id)
    .bind(req.range.from as f64 / 1000.0)
    .bind(req.range.to as f64 / 1000.0)
    .fetch_all(&st.postgres)
    .await?;
    let end = req.range.to.min(now_ms());
    // Nothing to show before the monitor existed.
    let start = req.range.from.max(m.row.created_at);
    let mut segments = Vec::new();
    let mut keys: Vec<String> = m.groups.iter().map(|g| g.group_key.clone()).collect();
    for e in &events {
        if !keys.contains(&e.group) {
            keys.push(e.group.clone());
        }
    }
    for key in keys {
        let mine: Vec<&MonitorEvent> = events.iter().filter(|e| e.group == key).collect();
        let current = m.groups.iter().find(|g| g.group_key == key).map(|g| MonitorStatus::parse(&g.status));
        let mut status = mine.first().map(|e| MonitorStatus::parse(&e.from_status)).or(current).unwrap_or(MonitorStatus::Ok);
        let mut from = start;
        for e in &mine {
            if e.created_at > from {
                segments.push(StatusSegment { group: key.clone(), status, from, to: e.created_at });
            }
            status = MonitorStatus::parse(&e.to_status);
            from = e.created_at.max(start);
        }
        if end > from {
            segments.push(StatusSegment { group: key.clone(), status, from, to: end });
        }
    }
    let mut events = events;
    events.reverse();
    events.truncate(500);
    Ok(Json(MonitorHistory { events, segments }))
}

// ================================================================ preview (backtest)

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorPreviewRequest {
    /// Metric (default) or log monitor.
    pub r#type: Option<MonitorType>,
    pub query: String,
    pub options: MonitorOptions,
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
pub struct PreviewSeries {
    pub group: String,
    /// `[timestamp ms, value]`; evaluated values are null while a window has no data.
    pub points: Vec<(i64, Option<f64>)>,
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransitionCounts {
    pub alert: i64,
    pub warn: i64,
    pub recover: i64,
    pub nodata: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorPreview {
    /// The query's raw series (largest 20 groups).
    pub raw: Vec<PreviewSeries>,
    /// What the monitor evaluates each minute (largest 20 groups).
    pub evaluated: Vec<PreviewSeries>,
    /// Transitions the monitor would have made over the range.
    pub transitions: TransitionCounts,
    pub groups: i64,
}

/// Backtests a monitor definition: how its value and status would have evolved over `range`.
#[utoipa::path(post, path = "/api/v1/monitors/preview", request_body = MonitorPreviewRequest, responses((status = 200, body = MonitorPreview)), tag = "monitors")]
pub async fn preview(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<MonitorPreviewRequest>) -> ApiResult<MonitorPreview> {
    req.range.validate()?;
    if req.range.to - req.range.from > 7 * DAY_MS {
        return Err(ApiError::BadRequest("previews cover at most a week".into()));
    }
    req.options.validate().map_err(ApiError::BadRequest)?;
    let span = req.range.to - req.range.from;
    let fetch = TimeRange { from: req.range.from - req.options.lookback_ms(), to: req.range.to };
    let points = ((fetch.to - fetch.from) / (span / 300).max(10_000)).clamp(10, 1000) as u32;
    let groups = monitor_series(&st, &org, &access, req.r#type.unwrap_or(MonitorType::Metric), &req.query, fetch, points).await?;
    let step = (span / 240).max(60_000);
    let b = backtest(&req.options, &groups, req.range.from, req.range.to, step);
    let mut transitions = TransitionCounts::default();
    for t in &b.transitions {
        match t.to {
            MonitorStatus::Alert => transitions.alert += 1,
            MonitorStatus::Warn => transitions.warn += 1,
            MonitorStatus::NoData => transitions.nodata += 1,
            MonitorStatus::Ok => transitions.recover += 1,
        }
    }
    // Chart the groups with the largest values.
    let peak = |g: &Group| g.points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|a, b| peak(&groups[*b]).total_cmp(&peak(&groups[*a])));
    order.truncate(20);
    let raw = order
        .iter()
        .map(|i| PreviewSeries {
            group: group_key(&groups[*i].tags),
            points: groups[*i].points.iter().filter(|p| p.0 >= req.range.from).map(|p| (p.0, Some(p.1))).collect(),
        })
        .collect();
    let evaluated = order.iter().map(|i| PreviewSeries { group: b.evaluated[*i].0.clone(), points: b.evaluated[*i].1.clone() }).collect();
    Ok(Json(MonitorPreview { raw, evaluated, transitions, groups: groups.len() as i64 }))
}

// ================================================================ quality

#[derive(Deserialize, ToSchema, Default)]
#[serde(default)]
pub struct MonitorQualityRequest {
    pub creators: Vec<String>,
    pub teams: Vec<String>,
    /// `service:` tag values.
    pub services: Vec<String>,
    /// `env:` tag values.
    pub envs: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct QualityMonitor {
    pub id: i64,
    pub name: String,
    pub r#type: MonitorType,
    /// What's wrong, e.g. "34 alerts daily".
    pub detail: String,
}

#[derive(Serialize, ToSchema)]
pub struct QualityIssue {
    /// high_volume | missing_recipients | muted_long | stuck_alert | stuck_no_data
    pub kind: String,
    pub title: String,
    pub description: String,
    pub fix: String,
    pub monitors: Vec<QualityMonitor>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonitorQuality {
    pub healthy: i64,
    pub to_improve: i64,
    pub issues: Vec<QualityIssue>,
    pub creators: Vec<MemberRef>,
    pub teams: Vec<String>,
    pub services: Vec<String>,
    pub envs: Vec<String>,
}

fn tag_values(tags: &[String], key: &str) -> Vec<String> {
    tags.iter().filter_map(|t| t.strip_prefix(key).and_then(|v| v.strip_prefix(':'))).map(str::to_string).collect()
}

#[utoipa::path(post, path = "/api/v1/monitors/quality", request_body = MonitorQualityRequest, responses((status = 200, body = MonitorQuality)), tag = "monitors")]
pub async fn quality(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<MonitorQualityRequest>) -> ApiResult<MonitorQuality> {
    let (loaded, downtimes) = load_all(&st, &org).await?;
    // Alerts per day over the last week, or over the monitor's history when it's younger than that.
    let alerts: Vec<(i64, f64)> = sqlx::query_as(
        "SELECT e.monitor_id, count(*) FILTER (WHERE e.to_status = 'alert')::float8 \
           / GREATEST(1, LEAST(7, extract(epoch FROM now() - min(e.created_at)) / 86400))::float8 \
         FROM monitor_events e JOIN monitors m ON m.id = e.monitor_id \
         WHERE m.org_id = $1 AND e.created_at > now() - interval '7 days' GROUP BY e.monitor_id",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let alerts: HashMap<i64, f64> = alerts.into_iter().collect();
    let now = now_ms();

    let mut creators: BTreeMap<String, String> = BTreeMap::new();
    let (mut teams, mut services, mut envs) = (HashSet::new(), HashSet::new(), HashSet::new());
    for m in &loaded {
        if let (Some(id), Some(n)) = (&m.row.creator_id, &m.row.creator_name) {
            creators.insert(id.clone(), n.clone());
        }
        teams.extend(m.row.teams.iter().cloned());
        services.extend(tag_values(&m.row.tags, "service"));
        envs.extend(tag_values(&m.row.tags, "env"));
    }
    let any = |want: &[String], have: Vec<String>| want.is_empty() || have.iter().any(|h| want.contains(h));
    let scoped: Vec<&Loaded> = loaded
        .iter()
        .filter(|m| {
            any(&req.creators, m.row.creator_id.iter().cloned().collect())
                && any(&req.teams, m.row.teams.clone())
                && any(&req.services, tag_values(&m.row.tags, "service"))
                && any(&req.envs, tag_values(&m.row.tags, "env"))
        })
        .collect();

    let q = |m: &Loaded, detail: String| QualityMonitor { id: m.row.id, name: m.row.name.clone(), r#type: MonitorType::parse(&m.row.kind), detail };
    // High volume: at least 5 alerts a day on average.
    let mut high: Vec<(f64, &Loaded)> = scoped.iter().filter_map(|m| alerts.get(&m.row.id).map(|d| (*d, *m))).filter(|(d, _)| *d >= 5.0).collect();
    high.sort_by(|a, b| b.0.total_cmp(&a.0));
    let missing: Vec<&Loaded> = scoped.iter().copied().filter(|m| !m.row.draft && m.recipients.is_empty()).collect();
    let muted_long: Vec<(i64, &Loaded)> = scoped
        .iter()
        .filter_map(|m| {
            m.muted?;
            let since = downtimes
                .iter()
                .filter(|d| d.applies_to(m.row.id, &m.row.tags) && d.window().active_at(now).is_some())
                .map(|d| d.window().active_at(now).map(|w| w.0).unwrap_or(now))
                .min()?;
            (now - since > 30 * DAY_MS).then_some((now - since, *m))
        })
        .collect();
    let stuck = |status: MonitorStatus, days: i64| -> Vec<&Loaded> {
        scoped.iter().copied().filter(|m| m.status == status && now - m.row.state_changed_at > days * DAY_MS).collect()
    };
    let stuck_alert = stuck(MonitorStatus::Alert, 7);
    let stuck_nodata = stuck(MonitorStatus::NoData, 1);

    let days = |ms: i64| ms / DAY_MS;
    let mut issues = Vec::new();
    let mut push = |kind: &str, title: String, description: &str, fix: &str, monitors: Vec<QualityMonitor>| {
        if !monitors.is_empty() {
            issues.push(QualityIssue { kind: kind.into(), title, description: description.into(), fix: fix.into(), monitors });
        }
    };
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    push(
        "high_volume",
        format!("{} generating a high volume of alerts", plural(high.len(), "monitor is", "monitors are")),
        "Monitors that alerted at least 5 times a day on average over the last week. Try adjusting the thresholds, evaluation window or recovery thresholds to improve accuracy and reduce noise.",
        "Resolve this issue by editing or deleting the monitor",
        high.iter().map(|(d, m)| q(m, format!("{} alerts daily", d.round() as i64))).collect(),
    );
    push(
        "missing_recipients",
        format!("{} missing recipients", plural(missing.len(), "monitor is", "monitors are")),
        "These monitors don't @-mention anyone in their message, so nobody is notified when they trigger.",
        "Add @handles to the monitor message",
        missing.iter().map(|m| q(m, "No recipients".into())).collect(),
    );
    push(
        "muted_long",
        format!("{} muted for more than 30 days", plural(muted_long.len(), "monitor has been", "monitors have been")),
        "Long-muted monitors give a false sense of coverage. Unmute them or delete them if they're no longer needed.",
        "Unmute or delete the monitor",
        muted_long.iter().map(|(ms, m)| q(m, format!("Muted for {} days", days(*ms)))).collect(),
    );
    push(
        "stuck_alert",
        format!("{} alerting for more than a week", plural(stuck_alert.len(), "monitor has been", "monitors have been")),
        "A monitor that never recovers is usually misconfigured or ignored.",
        "Review the thresholds or fix the underlying issue",
        stuck_alert.iter().map(|m| q(m, format!("Alerting for {} days", days(now - m.row.state_changed_at)))).collect(),
    );
    push(
        "stuck_no_data",
        format!("{} reporting No Data for more than a day", plural(stuck_nodata.len(), "monitor has been", "monitors have been")),
        "The query returns no data: the metric may have been renamed, or its tags no longer match.",
        "Fix the monitor query",
        stuck_nodata.iter().map(|m| q(m, format!("No data for {} days", days(now - m.row.state_changed_at).max(1)))).collect(),
    );
    let flagged: HashSet<i64> = issues.iter().flat_map(|i| i.monitors.iter().map(|m| m.id)).collect();
    let sorted = |s: HashSet<String>| {
        let mut v: Vec<String> = s.into_iter().collect();
        v.sort();
        v
    };
    Ok(Json(MonitorQuality {
        healthy: scoped.iter().filter(|m| !flagged.contains(&m.row.id)).count() as i64,
        to_improve: flagged.len() as i64,
        issues,
        creators: creators.into_iter().map(|(id, name)| MemberRef { id, name }).collect(),
        teams: sorted(teams),
        services: sorted(services),
        envs: sorted(envs),
    }))
}

// ================================================================ evaluation

/// Evaluates every monitor once a minute. One server at a time (advisory lock), so replicas don't
/// record each state change twice.
pub async fn run_evaluator(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let mut conn = match st.postgres.acquire().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "monitor evaluation: postgres unavailable");
                continue;
            }
        };
        if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock(7301)")
            .fetch_one(&mut *conn)
            .await
            .unwrap_or(false)
        {
            continue;
        }
        match sqlx::query_scalar::<_, i64>("SELECT id FROM monitors WHERE type <> 'synthetics' ORDER BY id")
            .fetch_all(&st.postgres)
            .await
        {
            Ok(ids) => {
                futures::stream::iter(ids)
                    .for_each_concurrent(8, |id| evaluate_one(&st, id))
                    .await
            }
            Err(e) => tracing::warn!(error = %e, "monitor evaluation: listing monitors failed"),
        }
        let _ = sqlx::query("SELECT pg_advisory_unlock(7301)")
            .execute(&mut *conn)
            .await;
    }
}

/// Monitors being evaluated right now (create/update evaluate immediately, racing the minute loop).
fn in_flight() -> &'static Mutex<HashSet<i64>> {
    static SET: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

pub async fn evaluate_one(st: &AppState, id: i64) {
    if !in_flight().lock().unwrap().insert(id) {
        return;
    }
    if let Err(e) = evaluate_inner(st, id, now_ms()).await {
        tracing::warn!(monitor = id, error = %e, "monitor evaluation failed");
    }
    in_flight().lock().unwrap().remove(&id);
}

async fn evaluate_inner(st: &AppState, id: i64, now: i64) -> Result<(), ApiError> {
    let Some(row): Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE m.id = $1 AND m.type <> 'synthetics'")).bind(id).fetch_optional(&st.postgres).await? else {
        return Ok(());
    };
    let groups: Vec<GroupRow> = sqlx::query_as(&format!("{GROUP_SELECT} WHERE g.monitor_id = $1")).bind(id).fetch_all(&st.postgres).await?;
    let downtimes = load_downtimes(st, &row.org_id).await?;
    let m = Loaded::new(row, groups, &downtimes, now);
    let Some(opts) = m.opts.clone() else { return Ok(()) };

    let range = TimeRange { from: now - opts.lookback_ms() - 60_000, to: now };
    let points = ((range.to - range.from) / 10_000).clamp(10, 1000) as u32;
    // A metric that doesn't exist (yet), or an unparsable query, is no data, not an error.
    let series = match monitor_series(st, &m.row.org_id, &LogAccess::org_background(), MonitorType::parse(&m.row.kind), &m.row.query, range, points).await {
        Ok(s) => s,
        Err(ApiError::BadRequest(_)) => Vec::new(),
        Err(e) => return Err(e),
    };
    let mut results: Vec<(String, BTreeMap<String, String>, Option<f64>, Option<i64>)> =
        series.iter().map(|g| (group_key(&g.tags), g.tags.clone(), opts.value_at(&g.points, now), g.points.last().map(|p| p.0))).collect();
    for g in &m.groups {
        if !results.iter().any(|r| r.0 == g.group_key) {
            results.push((g.group_key.clone(), parse_group_key(&g.group_key), None, None));
        }
    }

    let mut tx = st.postgres.begin().await?;
    let mut statuses = Vec::new();
    for (key, tags, value, last_point) in results {
        let existing = m.groups.iter().find(|g| g.group_key == key);
        let prev = existing.map(|g| MonitorStatus::parse(&g.status)).unwrap_or(MonitorStatus::Ok);
        let last_data = last_point.or(existing.and_then(|g| g.last_data_at));
        // Missing data only counts once it has been missing for the no-data timeframe.
        let fresh = last_data.is_some_and(|t| now - t < opts.no_data_minutes * 60_000);
        let next = if value.is_none() && fresh { prev } else { opts.classify(value, prev) };
        statuses.push(next);
        sqlx::query(
            "INSERT INTO monitor_groups (monitor_id, group_key, status, value, last_data_at, state_changed_at) VALUES ($1, $2, $3, $4, to_timestamp($5), now()) \
             ON CONFLICT (monitor_id, group_key) DO UPDATE SET status = EXCLUDED.status, value = COALESCE(EXCLUDED.value, monitor_groups.value), \
             last_data_at = COALESCE(EXCLUDED.last_data_at, monitor_groups.last_data_at), \
             state_changed_at = CASE WHEN monitor_groups.status = EXCLUDED.status THEN monitor_groups.state_changed_at ELSE now() END",
        )
        .bind(id)
        .bind(&key)
        .bind(next.as_str())
        .bind(value)
        .bind(last_data.map(|t| t as f64 / 1000.0))
        .execute(&mut *tx)
        .await?;
        if next != prev && !(existing.is_none() && next == MonitorStatus::Ok) {
            let muted = m.row.draft || m.group_muted(&key, &downtimes, now).is_some();
            insert_event(&mut tx, &m.row, &opts, &key, &tags, prev, next, value, muted, None).await?;
        }
    }
    let overall = worst(statuses).unwrap_or(MonitorStatus::NoData);
    sqlx::query(
        "UPDATE monitors SET status = $2, evaluated_at = now(), state_changed_at = CASE WHEN status = $2 THEN state_changed_at ELSE now() END WHERE id = $1",
    )
    .bind(id)
    .bind(overall.as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &Row,
    opts: &MonitorOptions,
    key: &str,
    tags: &BTreeMap<String, String>,
    from: MonitorStatus,
    to: MonitorStatus,
    value: Option<f64>,
    muted: bool,
    at: Option<i64>,
) -> Result<(), ApiError> {
    let ctx = MessageContext { status: to, previous: from, value, opts, monitor_name: &row.name, group: tags };
    let message = render_message(&row.message, &ctx);
    let who = if muted { Vec::new() } else { recipients(&message) };
    let (event_id,): (i64,) = sqlx::query_as(
        "INSERT INTO monitor_events (monitor_id, group_key, from_status, to_status, value, muted, recipients, message, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, COALESCE(to_timestamp($9), now())) RETURNING id",
    )
    .bind(row.id)
    .bind(key)
    .bind(from.as_str())
    .bind(to.as_str())
    .bind(value)
    .bind(muted)
    .bind(&who)
    .bind(&message)
    .bind(at.map(|t| t as f64 / 1000.0))
    .fetch_one(&mut **tx)
    .await?;
    // Live transitions notify; replayed history (backfill) doesn't.
    if at.is_none() {
        for r in &who {
            sqlx::query("INSERT INTO notification_deliveries (event_id, org_id, recipient) VALUES ($1, $2, $3)").bind(event_id).bind(&row.org_id).bind(r).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

/// Replays a new monitor over the last day so its history, status bars and the quality report
/// have something to show (used by the demo seed).
pub(crate) async fn backfill(st: &AppState, id: i64) -> Result<(), ApiError> {
    let row: Row = sqlx::query_as(&format!("{SELECT} WHERE m.id = $1")).bind(id).fetch_one(&st.postgres).await?;
    let Some(opts) = serde_json::from_value::<MonitorOptions>(row.definition.clone()).ok() else { return Ok(()) };
    let now = now_ms();
    let from = now - DAY_MS;
    let range = TimeRange { from: from - opts.lookback_ms(), to: now };
    let series = match monitor_series(st, &row.org_id, &LogAccess::org_background(), MonitorType::parse(&row.kind), &row.query, range, 1000).await {
        Ok(s) => s,
        Err(ApiError::BadRequest(_)) => return Ok(()),
        Err(e) => return Err(e),
    };
    let b = backtest(&opts, &series, from, now, 60_000);
    let mut tx = st.postgres.begin().await?;
    let downtimes = load_downtimes(st, &row.org_id).await?;
    let m = Loaded::new(row.clone(), Vec::new(), &downtimes, now);
    for t in &b.transitions {
        let tags = parse_group_key(&t.group);
        let muted = row.draft || m.group_muted(&t.group, &downtimes, t.at).is_some();
        insert_event(&mut tx, &row, &opts, &t.group, &tags, t.from, t.to, t.value, muted, Some(t.at)).await?;
    }
    for (key, status, value) in &b.last {
        let changed = b.transitions.iter().rev().find(|t| &t.group == key).map(|t| t.at).unwrap_or(from);
        sqlx::query(
            "INSERT INTO monitor_groups (monitor_id, group_key, status, value, last_data_at, state_changed_at) VALUES ($1, $2, $3, $4, now(), to_timestamp($5)) \
             ON CONFLICT (monitor_id, group_key) DO NOTHING",
        )
        .bind(id)
        .bind(key)
        .bind(status.as_str())
        .bind(value)
        .bind(changed as f64 / 1000.0)
        .execute(&mut *tx)
        .await?;
    }
    let overall = worst(b.last.iter().map(|l| l.1)).unwrap_or(MonitorStatus::NoData);
    let changed = b.transitions.last().map(|t| t.at).unwrap_or(from);
    sqlx::query("UPDATE monitors SET status = $2, state_changed_at = to_timestamp($3), evaluated_at = now() WHERE id = $1")
        .bind(id)
        .bind(overall.as_str())
        .bind(changed as f64 / 1000.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Monitor summary widget: counts and the monitors themselves, worst first.
pub(crate) async fn summary(st: &AppState, org: &str, text: &str) -> Result<Vec<(i64, String, MonitorStatus, bool)>, ApiError> {
    let (loaded, _) = load_all(st, org).await?;
    let text = text.to_lowercase();
    let mut out: Vec<(i64, String, MonitorStatus, bool)> =
        loaded.into_iter().filter(|m| text.is_empty() || m.row.name.to_lowercase().contains(&text)).map(|m| (m.row.id, m.row.name.clone(), m.status, m.muted.is_some())).collect();
    out.sort_by(|a, b| b.2.severity().cmp(&a.2.severity()).then_with(|| a.1.cmp(&b.1)));
    out.truncate(500);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: &str, query: &str) -> MonitorInput {
        serde_json::from_value(serde_json::json!({
            "name": "m", "type": kind, "query": query, "priority": null,
            "options": {"detection": "threshold", "timeAggregation": "max", "windowMinutes": 15, "comparator": "above_or_equal",
                        "thresholds": {"critical": 2.0, "warning": null, "criticalRecovery": null, "warningRecovery": null}}
        }))
        .unwrap()
    }

    #[test]
    fn metric_monitors_accept_arithmetic_between_queries() {
        assert!(validate(&mut input("metric", "sum:a.desired{*} by {d} - sum:a.ready{*} by {d}")).is_ok());
        assert!(validate(&mut input("metric", "default_zero(sum:n{s:ok} by {c} * 100 / sum:n{*} by {c})")).is_ok());
        assert!(validate(&mut input("metric", "sum:a{*} -")).is_err());
        // APM monitors: every query is a trace metric.
        assert!(validate(&mut input("apm", "sum:trace.http.request.errors{*} / sum:trace.http.request.hits{*}")).is_ok());
        assert!(validate(&mut input("apm", "sum:trace.http.request.errors{*} / sum:system.load.1{*}")).is_err());
    }
}
