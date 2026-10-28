//! Service level objectives (Monitoring → Service Level Objectives): definitions and corrections in
//! Postgres, SLIs computed on request. A metric SLO counts good over total events from two metric
//! queries, a monitor SLO the time none of its monitors alerted (from `monitor_events`, which the
//! server keeps for as long as the monitor exists) and a time-slice SLO the share of slices whose
//! value passed a condition. The math is in [`super::slo_math`].
//!
//! Each SLO is measured once per window and cached for a minute per org (cleared on this server by
//! every change); lists are filtered and faceted in memory, as an org has at most a few hundred SLOs.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use futures::StreamExt;
use oc_query::parse_metric_query;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use super::identity::MemberRef;
use super::logs::TimeRange;
use super::metrics::evaluate_query_stepped;
use super::monitor_formula::{combine, parse_formula, Expr};
use super::slo_math::{self as math, Measure, Span, Transition};
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const MAX_SLOS: i64 = 500;
const MAX_NAME_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 2000;
const MAX_QUERY_CHARS: usize = 2000;
const MAX_MONITORS: usize = 20;
const MAX_TAGS: usize = 50;
const MAX_CORRECTION_DAYS: i64 = 90;
const MIN_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
/// Points per window for metric SLOs: 2-minute buckets over a day, 3-hour ones over 90 days.
const WINDOW_POINTS: u32 = 720;
/// At most this many time slices per window (longer windows get longer slices).
const MAX_SLICES: i64 = 10_080;
/// Monitor transitions read per SLO and window.
const MAX_TRANSITIONS: i64 = 100_000;
/// Cache lifetime of a measured window.
const CACHE_SECS: u64 = 60;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ================================================================ types

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SloType {
    /// Good events over total events, from two metric queries.
    Metric,
    /// Time none of the monitors alerted.
    Monitor,
    /// Share of time slices whose value passed a condition.
    TimeSlice,
}

impl SloType {
    fn as_str(self) -> &'static str {
        match self {
            SloType::Metric => "metric",
            SloType::Monitor => "monitor",
            SloType::TimeSlice => "time_slice",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "monitor" => SloType::Monitor,
            "time_slice" => SloType::TimeSlice,
            _ => SloType::Metric,
        }
    }
}

/// Rolling time window an SLO is measured over.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum SloWindow {
    #[serde(rename = "1d")]
    OneDay,
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
    #[serde(rename = "90d")]
    NinetyDays,
}

impl SloWindow {
    fn ms(self) -> i64 {
        DAY_MS
            * match self {
                SloWindow::OneDay => 1,
                SloWindow::SevenDays => 7,
                SloWindow::ThirtyDays => 30,
                SloWindow::NinetyDays => 90,
            }
    }
    fn as_str(self) -> &'static str {
        match self {
            SloWindow::OneDay => "1d",
            SloWindow::SevenDays => "7d",
            SloWindow::ThirtyDays => "30d",
            SloWindow::NinetyDays => "90d",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        [SloWindow::OneDay, SloWindow::SevenDays, SloWindow::ThirtyDays, SloWindow::NinetyDays].into_iter().find(|w| w.as_str() == s)
    }
    /// Bucket of the SLI and error budget charts: a multiple of the metric points' interval.
    fn history_bucket(self) -> i64 {
        match self {
            SloWindow::OneDay => 30 * MIN_MS,
            SloWindow::SevenDays => 2 * HOUR_MS,
            SloWindow::ThirtyDays => 6 * HOUR_MS,
            SloWindow::NinetyDays => DAY_MS,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SloComparator {
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
}

impl SloComparator {
    fn as_str(self) -> &'static str {
        match self {
            SloComparator::Lt => "<",
            SloComparator::Le => "<=",
            SloComparator::Gt => ">",
            SloComparator::Ge => ">=",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        [SloComparator::Lt, SloComparator::Le, SloComparator::Gt, SloComparator::Ge].into_iter().find(|c| c.as_str() == s)
    }
    fn math(self) -> math::Cmp {
        match self {
            SloComparator::Lt => math::Cmp::Lt,
            SloComparator::Le => math::Cmp::Le,
            SloComparator::Gt => math::Cmp::Gt,
            SloComparator::Ge => math::Cmp::Ge,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Debug)]
pub struct SloTarget {
    pub window: SloWindow,
    /// Percent, e.g. 99.9.
    pub target: f64,
    /// Percent above the target: below it the SLO is in warning.
    pub warning: Option<f64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SloState {
    Ok,
    Warning,
    Breached,
    NoData,
}

impl SloState {
    fn as_str(self) -> &'static str {
        match self {
            SloState::Ok => "ok",
            SloState::Warning => "warning",
            SloState::Breached => "breached",
            SloState::NoData => "no_data",
        }
    }
    /// Sort order: worst first.
    fn severity(self) -> u8 {
        match self {
            SloState::Breached => 3,
            SloState::Warning => 2,
            SloState::NoData => 1,
            SloState::Ok => 0,
        }
    }
}

impl From<math::State> for SloState {
    fn from(s: math::State) -> Self {
        match s {
            math::State::Ok => SloState::Ok,
            math::State::Warning => SloState::Warning,
            math::State::Breached => SloState::Breached,
            math::State::NoData => SloState::NoData,
        }
    }
}

/// What the error budget is counted in.
#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BudgetUnit {
    Events,
    Minutes,
}

/// How fast the error budget burns over the last hour.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BurnLevel {
    /// 10× or more: the whole budget goes in a tenth of the window.
    High,
    /// 1× to 10×.
    Elevated,
    /// Under 1×.
    Normal,
    /// No data in the last hour.
    Unknown,
}

impl BurnLevel {
    fn of(rate: Option<f64>) -> Self {
        match rate {
            None => BurnLevel::Unknown,
            Some(r) if r >= 10.0 => BurnLevel::High,
            Some(r) if r >= 1.0 => BurnLevel::Elevated,
            Some(_) => BurnLevel::Normal,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            BurnLevel::High => "high",
            BurnLevel::Elevated => "elevated",
            BurnLevel::Normal => "normal",
            BurnLevel::Unknown => "unknown",
        }
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SloStatus {
    pub window: SloWindow,
    pub target: f64,
    pub warning: Option<f64>,
    pub state: SloState,
    /// Percent of good events (metric) or good time (monitor, time slice); absent without data.
    pub sli: Option<f64>,
    /// Good and total events, or minutes, over the window (corrections excluded).
    pub good: f64,
    pub total: f64,
    pub unit: BudgetUnit,
    /// Error budget left, in percent of the window's budget (negative once overspent).
    pub budget_remaining: Option<f64>,
    /// Bad events (or minutes) the target allows over the window, and how many were spent.
    pub budget_allowed: Option<f64>,
    pub budget_used: Option<f64>,
    /// Bad share over the share the target allows: over the last hour, and over the window.
    pub burn_rate_1h: Option<f64>,
    pub burn_rate: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SloItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub r#type: SloType,
    /// Metric SLOs: metric queries (or formulas) of the good and the total events.
    pub good_query: Option<String>,
    pub total_query: Option<String>,
    /// Monitor SLOs.
    pub monitor_ids: Vec<i64>,
    /// Time-slice SLOs: a slice is good when `sliceQuery comparator threshold`.
    pub slice_query: Option<String>,
    pub comparator: Option<SloComparator>,
    pub threshold: Option<f64>,
    pub slice_minutes: Option<i32>,
    pub targets: Vec<SloTarget>,
    pub primary_window: SloWindow,
    pub tags: Vec<String>,
    pub teams: Vec<String>,
    pub creator: Option<MemberRef>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Over the requested window (the primary one by default).
    pub status: SloStatus,
}

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SloInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub r#type: SloType,
    pub good_query: Option<String>,
    pub total_query: Option<String>,
    #[serde(default)]
    pub monitor_ids: Vec<i64>,
    pub slice_query: Option<String>,
    pub comparator: Option<SloComparator>,
    pub threshold: Option<f64>,
    /// 1 or 5.
    pub slice_minutes: Option<i32>,
    /// One per window.
    pub targets: Vec<SloTarget>,
    /// One of the targets' windows; absent = the first target's.
    pub primary_window: Option<SloWindow>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub teams: Vec<String>,
}

fn clean_list(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    for v in values.iter_mut() {
        *v = v.trim().to_string();
    }
    values.retain(|v| !v.is_empty() && seen.insert(v.clone()));
}

fn check_query(label: &str, q: &mut Option<String>) -> Result<(), String> {
    let text = q.as_deref().map(str::trim).unwrap_or("").to_string();
    if text.is_empty() {
        return Err(format!("The {label} query is required"));
    }
    if text.len() > MAX_QUERY_CHARS {
        return Err(format!("The {label} query is at most {MAX_QUERY_CHARS} characters"));
    }
    parse_formula(&text).map_err(|e| format!("Invalid {label} query: {e}"))?;
    *q = Some(text);
    Ok(())
}

/// Trims and checks an input, keeping only the fields of its type.
fn validate(input: &mut SloInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    input.description = input.description.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Name must be 1–{MAX_NAME_CHARS} characters"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    if input.r#type == SloType::Metric {
        check_query("good events", &mut input.good_query)?;
        check_query("total events", &mut input.total_query)?;
    } else {
        (input.good_query, input.total_query) = (None, None);
    }
    if input.r#type == SloType::Monitor {
        input.monitor_ids.sort_unstable();
        input.monitor_ids.dedup();
        if input.monitor_ids.is_empty() || input.monitor_ids.len() > MAX_MONITORS {
            return Err(format!("Pick 1–{MAX_MONITORS} monitors"));
        }
    } else {
        input.monitor_ids.clear();
    }
    if input.r#type == SloType::TimeSlice {
        check_query("time slice", &mut input.slice_query)?;
        if input.comparator.is_none() || !input.threshold.is_some_and(f64::is_finite) {
            return Err("A time slice needs a comparator and a numeric threshold".into());
        }
        if !matches!(input.slice_minutes, Some(1 | 5)) {
            return Err("Time slices are 1 or 5 minutes".into());
        }
    } else {
        (input.slice_query, input.comparator, input.threshold, input.slice_minutes) = (None, None, None, None);
    }
    if input.targets.is_empty() || input.targets.len() > 4 {
        return Err("Set a target for 1–4 time windows".into());
    }
    let mut windows: Vec<SloWindow> = input.targets.iter().map(|t| t.window).collect();
    windows.sort();
    windows.dedup();
    if windows.len() != input.targets.len() {
        return Err("Set one target per time window".into());
    }
    for t in &input.targets {
        if !(t.target.is_finite() && t.target > 0.0 && t.target < 100.0) {
            return Err("Targets are percentages above 0 and below 100".into());
        }
        if t.warning.is_some_and(|w| !(w.is_finite() && w > t.target && w < 100.0)) {
            return Err("A warning threshold is above its target and below 100".into());
        }
    }
    let primary = input.primary_window.unwrap_or(input.targets[0].window);
    if !windows.contains(&primary) {
        return Err("The primary time window needs a target".into());
    }
    input.primary_window = Some(primary);
    input.targets.sort_by_key(|t| t.window);
    clean_list(&mut input.tags);
    clean_list(&mut input.teams);
    if input.tags.len() > MAX_TAGS || input.teams.len() > MAX_TAGS {
        return Err(format!("At most {MAX_TAGS} tags and {MAX_TAGS} teams"));
    }
    Ok(())
}

/// The target for `window`: its own, or the primary target measured over `window`.
fn target_for(targets: &[SloTarget], window: SloWindow, primary: SloWindow) -> SloTarget {
    targets.iter().find(|t| t.window == window).or_else(|| targets.iter().find(|t| t.window == primary)).or(targets.first()).map(|t| SloTarget { window, ..*t }).unwrap_or(SloTarget {
        window,
        target: 99.9,
        warning: None,
    })
}

// ================================================================ storage

#[derive(sqlx::FromRow, Clone)]
struct Row {
    id: String,
    name: String,
    description: String,
    kind: String,
    good_query: Option<String>,
    total_query: Option<String>,
    monitor_ids: Vec<i64>,
    slice_query: Option<String>,
    comparator: Option<String>,
    threshold: Option<f64>,
    slice_minutes: Option<i32>,
    targets: Value,
    primary_window: String,
    tags: Vec<String>,
    teams: Vec<String>,
    creator_id: Option<String>,
    creator_name: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const SELECT: &str = "SELECT s.id::text AS id, s.name, s.description, s.type AS kind, s.good_query, s.total_query, s.monitor_ids, s.slice_query, s.comparator, \
    s.threshold, s.slice_minutes, s.targets, s.primary_window, s.tags, s.teams, s.created_by::text AS creator_id, u.name AS creator_name, \
    (extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM s.updated_at) * 1000)::bigint AS updated_at \
    FROM slos s LEFT JOIN users u ON u.id = s.created_by";

impl Row {
    fn kind(&self) -> SloType {
        SloType::parse(&self.kind)
    }
    fn targets(&self) -> Vec<SloTarget> {
        serde_json::from_value(self.targets.clone()).unwrap_or_default()
    }
    fn primary(&self) -> SloWindow {
        SloWindow::parse(&self.primary_window).unwrap_or(SloWindow::ThirtyDays)
    }
    fn item(&self, status: SloStatus) -> SloItem {
        SloItem {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            r#type: self.kind(),
            good_query: self.good_query.clone(),
            total_query: self.total_query.clone(),
            monitor_ids: self.monitor_ids.clone(),
            slice_query: self.slice_query.clone(),
            comparator: self.comparator.as_deref().and_then(SloComparator::parse),
            threshold: self.threshold,
            slice_minutes: self.slice_minutes,
            targets: self.targets(),
            primary_window: self.primary(),
            tags: self.tags.clone(),
            teams: self.teams.clone(),
            creator: self.creator_id.clone().map(|id| MemberRef { id, name: self.creator_name.clone().unwrap_or_default() }),
            created_at: self.created_at,
            updated_at: self.updated_at,
            status,
        }
    }
}

async fn load_one(st: &AppState, org: &str, id: &str) -> Result<Row, ApiError> {
    sqlx::query_as(&format!("{SELECT} WHERE s.org_id = $1 AND s.id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?.ok_or_else(|| ApiError::NotFound(format!("SLO {id} not found")))
}

#[derive(sqlx::FromRow, Clone)]
struct CorrectionRow {
    id: String,
    slo_id: Option<String>,
    slo_name: Option<String>,
    category: String,
    description: String,
    start_at: i64,
    end_at: i64,
    creator_id: Option<String>,
    creator_name: Option<String>,
    created_at: i64,
}

const CORRECTION_SELECT: &str = "SELECT c.id::text AS id, c.slo_id::text AS slo_id, s.name AS slo_name, c.category, c.description, \
    (extract(epoch FROM c.start_at) * 1000)::bigint AS start_at, (extract(epoch FROM c.end_at) * 1000)::bigint AS end_at, \
    c.created_by::text AS creator_id, u.name AS creator_name, (extract(epoch FROM c.created_at) * 1000)::bigint AS created_at \
    FROM slo_corrections c LEFT JOIN slos s ON s.org_id = c.org_id AND s.id = c.slo_id LEFT JOIN users u ON u.id = c.created_by";

/// The org's corrections that end after `since`.
async fn load_corrections(st: &AppState, org: &str, since: i64) -> Result<Vec<CorrectionRow>, ApiError> {
    Ok(sqlx::query_as(&format!("{CORRECTION_SELECT} WHERE c.org_id = $1 AND c.end_at > to_timestamp($2) ORDER BY c.start_at DESC"))
        .bind(org)
        .bind(since as f64 / 1000.0)
        .fetch_all(&st.postgres)
        .await?)
}

/// Spans of the corrections that apply to an SLO (its own and the org-wide ones).
fn correction_spans(corrections: &[CorrectionRow], slo_id: &str) -> Vec<Span> {
    math::merge(corrections.iter().filter(|c| c.slo_id.as_deref().is_none_or(|s| s == slo_id)).map(|c| (c.start_at, c.end_at)).collect())
}

// ================================================================ measuring

/// What an SLO measured over a window ending now.
#[derive(Clone)]
struct Measured {
    /// Good and total amounts per chart bucket.
    buckets: Vec<(i64, Measure)>,
    last_hour: Measure,
    unit: BudgetUnit,
}

type CacheKey = (String, String, SloWindow);

fn cache() -> &'static Mutex<HashMap<CacheKey, (Measured, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, (Measured, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Drops the org's cached measures (after any SLO or correction change on this server).
fn forget(org: &str) {
    cache().lock().unwrap().retain(|k, _| k.0 != org);
}

/// A metric query or formula added up over its groups, and the interval (ms) of its points.
async fn metric_series(st: &AppState, org: &str, text: &str, range: TimeRange, points: u32) -> Result<(BTreeMap<i64, f64>, i64), ApiError> {
    let f = parse_formula(text).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
    let mut results = Vec::with_capacity(f.queries.len());
    let mut step = 0;
    for q in &f.queries {
        let q = parse_metric_query(q).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
        let (groups, s) = evaluate_query_stepped(st, org, &q, range, points).await?;
        step = step.max(s);
        results.push(groups);
    }
    let groups = if f.expr == Expr::Query(0) { results.remove(0) } else { combine(&f.expr, &results) };
    Ok((math::sum_series(groups.iter().map(|g| g.points.as_slice())), step))
}

async fn metric_measures(st: &AppState, org: &str, row: &Row, range: TimeRange, points: u32, corrections: &[Span]) -> Result<Vec<(i64, Measure)>, ApiError> {
    let (good, total) = (row.good_query.as_deref().unwrap_or_default(), row.total_query.as_deref().unwrap_or_default());
    let ((good, _), (total, step)) = futures::try_join!(metric_series(st, org, good, range, points), metric_series(st, org, total, range, points))?;
    Ok(math::metric_measures(&good, &total, step, range.from, range.to, corrections))
}

async fn slice_measures(st: &AppState, org: &str, row: &Row, range: TimeRange, corrections: &[Span]) -> Result<Vec<(i64, Measure)>, ApiError> {
    let slice = row.slice_minutes.unwrap_or(5).max(1) as i64 * MIN_MS;
    let points = ((range.to - range.from) / slice).clamp(1, MAX_SLICES) as u32;
    let (series, step) = metric_series(st, org, row.slice_query.as_deref().unwrap_or_default(), range, points).await?;
    let cmp = row.comparator.as_deref().and_then(SloComparator::parse).unwrap_or(SloComparator::Lt).math();
    Ok(math::slice_measures(&series, step, cmp, row.threshold.unwrap_or(0.0), range.from, range.to, corrections))
}

#[derive(sqlx::FromRow)]
struct TransitionRow {
    monitor_id: i64,
    group_key: String,
    from_status: String,
    to_status: String,
    at: i64,
}

/// When any of a monitor SLO's monitors alerted within the range, and since when they exist.
async fn monitor_down(st: &AppState, org: &str, row: &Row, range: TimeRange) -> Result<(Vec<Span>, Option<i64>), ApiError> {
    let monitors: Vec<(i64, i64)> =
        sqlx::query_as("SELECT id, (extract(epoch FROM created_at) * 1000)::bigint FROM monitors WHERE org_id = $1 AND id = ANY($2)").bind(org).bind(&row.monitor_ids).fetch_all(&st.postgres).await?;
    let ids: Vec<i64> = monitors.iter().map(|m| m.0).collect();
    let events: Vec<TransitionRow> = sqlx::query_as(
        "SELECT e.monitor_id, e.group_key, e.from_status, e.to_status, (extract(epoch FROM e.created_at) * 1000)::bigint AS at \
         FROM monitor_events e JOIN monitors m ON m.id = e.monitor_id \
         WHERE m.org_id = $1 AND e.monitor_id = ANY($2) AND e.created_at >= to_timestamp($3) AND e.created_at <= to_timestamp($4) \
         ORDER BY e.created_at LIMIT $5",
    )
    .bind(org)
    .bind(&ids)
    .bind(range.from as f64 / 1000.0)
    .bind(range.to as f64 / 1000.0)
    .bind(MAX_TRANSITIONS)
    .fetch_all(&st.postgres)
    .await?;
    let alerting: Vec<(i64, String)> =
        sqlx::query_as("SELECT g.monitor_id, g.group_key FROM monitor_groups g JOIN monitors m ON m.id = g.monitor_id WHERE m.org_id = $1 AND g.monitor_id = ANY($2) AND g.status = 'alert'")
            .bind(org)
            .bind(&ids)
            .fetch_all(&st.postgres)
            .await?;
    let mut down = Vec::new();
    for id in &ids {
        let transitions: Vec<Transition> = events
            .iter()
            .filter(|e| e.monitor_id == *id)
            .map(|e| Transition { group: e.group_key.clone(), at: e.at, from_alert: e.from_status == "alert", to_alert: e.to_status == "alert" })
            .collect();
        let now: Vec<String> = alerting.iter().filter(|a| a.0 == *id).map(|a| a.1.clone()).collect();
        down.extend(math::alert_spans(&transitions, &now, range.from, range.to));
    }
    Ok((math::merge(down), monitors.iter().map(|m| m.1).min()))
}

/// Measures an SLO over `window` (ending now), cached for a minute.
async fn measure(st: &AppState, org: &str, row: &Row, window: SloWindow, corrections: &[Span]) -> Result<Measured, ApiError> {
    let key = (org.to_string(), row.id.clone(), window);
    if let Some((m, at)) = cache().lock().unwrap().get(&key) {
        if at.elapsed() < Duration::from_secs(CACHE_SECS) {
            return Ok(m.clone());
        }
    }
    let now = now_ms();
    let range = TimeRange { from: now - window.ms(), to: now };
    let hour = TimeRange { from: now - HOUR_MS, to: now };
    let bucket = window.history_bucket();
    let measured = match row.kind() {
        SloType::Metric => {
            let (all, last) = futures::try_join!(metric_measures(st, org, row, range, WINDOW_POINTS, corrections), metric_measures(st, org, row, hour, 60, corrections))?;
            Measured { buckets: math::rebucket(&all, range.from, bucket), last_hour: math::total(&last), unit: BudgetUnit::Events }
        }
        SloType::TimeSlice => {
            let all = slice_measures(st, org, row, range, corrections).await?;
            Measured { buckets: math::rebucket(&all, range.from, bucket), last_hour: math::total_since(&all, hour.from), unit: BudgetUnit::Minutes }
        }
        SloType::Monitor => {
            let (down, since) = monitor_down(st, org, row, range).await?;
            let observed = since.map_or((range.to, range.to), |s| (s.max(range.from), range.to));
            Measured {
                buckets: math::uptime_measures(&down, observed, corrections, range.from, range.to, bucket),
                last_hour: math::total(&math::uptime_measures(&down, observed, corrections, hour.from, hour.to, HOUR_MS)),
                unit: BudgetUnit::Minutes,
            }
        }
    };
    cache().lock().unwrap().insert(key, (measured.clone(), Instant::now()));
    Ok(measured)
}

/// The SLO's status from what it measured: minutes for time-based SLOs.
fn status(m: &Measured, target: SloTarget) -> SloStatus {
    let scale = if m.unit == BudgetUnit::Minutes { 1.0 / MIN_MS as f64 } else { 1.0 };
    let window = math::total(&m.buckets);
    let sli = math::sli(window);
    let budget = math::budget(window, target.target);
    SloStatus {
        window: target.window,
        target: target.target,
        warning: target.warning,
        state: math::state(sli, target.target, target.warning).into(),
        sli,
        good: window.good * scale,
        total: window.total * scale,
        unit: m.unit,
        budget_remaining: budget.map(|b| b.remaining_pct),
        budget_allowed: budget.map(|b| b.allowed * scale),
        budget_used: budget.map(|b| b.used * scale),
        burn_rate_1h: math::burn_rate(m.last_hour, target.target),
        burn_rate: math::burn_rate(window, target.target),
    }
}

fn no_data(target: SloTarget, kind: SloType) -> SloStatus {
    let unit = if kind == SloType::Metric { BudgetUnit::Events } else { BudgetUnit::Minutes };
    status(&Measured { buckets: Vec::new(), last_hour: Measure::default(), unit }, target)
}

/// The SLO's status over `window`; a query that fails (e.g. a metric that never reported) is no data.
async fn status_over(st: &AppState, org: &str, row: &Row, window: SloWindow, corrections: &[Span]) -> (SloStatus, Option<Measured>) {
    let target = target_for(&row.targets(), window, row.primary());
    match measure(st, org, row, window, corrections).await {
        Ok(m) => (status(&m, target), Some(m)),
        Err(e) => {
            tracing::debug!(slo = %row.id, error = %e, "SLO has no data");
            (no_data(target, row.kind()), None)
        }
    }
}

// ================================================================ list

#[derive(Serialize, ToSchema, Clone)]
pub struct SloFacetCount {
    pub value: String,
    pub count: i64,
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SloSearchRequest {
    /// Words match name, description and tags; `key:value` words match tags (`service:billing`).
    pub query: String,
    pub types: Vec<SloType>,
    pub states: Vec<SloState>,
    pub burn_rates: Vec<BurnLevel>,
    /// Creator user ids.
    pub creators: Vec<String>,
    pub teams: Vec<String>,
    /// Values of the `service:` and `env:` tags.
    pub services: Vec<String>,
    pub envs: Vec<String>,
    /// Other tags (`key:value`).
    pub tags: Vec<String>,
    /// Window the statuses are computed over; absent = each SLO's primary window.
    pub window: Option<SloWindow>,
    /// state (default) | name | sli | budget | modified
    pub sort: Option<String>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SloFacets {
    pub r#type: Vec<SloFacetCount>,
    pub state: Vec<SloFacetCount>,
    pub burn_rate: Vec<SloFacetCount>,
    /// `id|name`.
    pub creators: Vec<SloFacetCount>,
    pub teams: Vec<SloFacetCount>,
    pub services: Vec<SloFacetCount>,
    pub envs: Vec<SloFacetCount>,
    pub tags: Vec<SloFacetCount>,
}

/// SLOs per state, every filter but the state applied.
#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SloSummary {
    pub breached: i64,
    pub warning: i64,
    pub ok: i64,
    pub no_data: i64,
    pub total: i64,
}

#[derive(Serialize, ToSchema)]
pub struct SloSearchPage {
    pub slos: Vec<SloItem>,
    pub total: i64,
    pub summary: SloSummary,
    pub facets: SloFacets,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Facet {
    Type,
    State,
    Burn,
    Creator,
    Team,
    Service,
    Env,
    Tag,
}

fn tag_values(tags: &[String], key: &str) -> Vec<String> {
    tags.iter().filter_map(|t| t.strip_prefix(key).and_then(|v| v.strip_prefix(':'))).map(str::to_string).collect()
}

fn other_tags(tags: &[String]) -> Vec<String> {
    tags.iter().filter(|t| !t.starts_with("service:") && !t.starts_with("env:")).cloned().collect()
}

fn text_matches(query: &str, item: &SloItem) -> bool {
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        if w.contains(':') && !w.starts_with(':') {
            return item.tags.iter().any(|t| t.to_lowercase() == w);
        }
        item.name.to_lowercase().contains(&w) || item.description.to_lowercase().contains(&w) || item.tags.iter().any(|t| t.to_lowercase().contains(&w))
    })
}

fn passes(i: &SloItem, req: &SloSearchRequest, skip: Option<Facet>) -> bool {
    let on = |f: Facet| skip != Some(f);
    let any = |want: &[String], have: Vec<String>| want.is_empty() || have.iter().any(|h| want.contains(h));
    (!on(Facet::Type) || req.types.is_empty() || req.types.contains(&i.r#type))
        && (!on(Facet::State) || req.states.is_empty() || req.states.contains(&i.status.state))
        && (!on(Facet::Burn) || req.burn_rates.is_empty() || req.burn_rates.contains(&BurnLevel::of(i.status.burn_rate_1h)))
        && (!on(Facet::Creator) || req.creators.is_empty() || i.creator.as_ref().is_some_and(|c| req.creators.contains(&c.id)))
        && (!on(Facet::Team) || any(&req.teams, i.teams.clone()))
        && (!on(Facet::Service) || any(&req.services, tag_values(&i.tags, "service")))
        && (!on(Facet::Env) || any(&req.envs, tag_values(&i.tags, "env")))
        && (!on(Facet::Tag) || any(&req.tags, other_tags(&i.tags)))
}

fn counts(values: impl Iterator<Item = String>) -> Vec<SloFacetCount> {
    let mut map: BTreeMap<String, i64> = BTreeMap::new();
    for v in values {
        *map.entry(v).or_default() += 1;
    }
    let mut out: Vec<SloFacetCount> = map.into_iter().map(|(value, count)| SloFacetCount { value, count }).collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    out
}

/// Facets (each ignoring its own filter), the state summary and the sorted page of matches.
fn search_page(items: Vec<SloItem>, req: &SloSearchRequest) -> SloSearchPage {
    let items: Vec<SloItem> = items.into_iter().filter(|i| text_matches(&req.query, i)).collect();
    let facet = |f: Facet, key: &dyn Fn(&SloItem) -> Vec<String>| counts(items.iter().filter(|i| passes(i, req, Some(f))).flat_map(key));
    let facets = SloFacets {
        r#type: facet(Facet::Type, &|i| vec![i.r#type.as_str().into()]),
        state: facet(Facet::State, &|i| vec![i.status.state.as_str().into()]),
        burn_rate: facet(Facet::Burn, &|i| vec![BurnLevel::of(i.status.burn_rate_1h).as_str().into()]),
        creators: facet(Facet::Creator, &|i| i.creator.iter().map(|c| format!("{}|{}", c.id, c.name)).collect()),
        teams: facet(Facet::Team, &|i| i.teams.clone()),
        services: facet(Facet::Service, &|i| tag_values(&i.tags, "service")),
        envs: facet(Facet::Env, &|i| tag_values(&i.tags, "env")),
        tags: facet(Facet::Tag, &|i| other_tags(&i.tags)),
    };
    let mut summary = SloSummary::default();
    for i in items.iter().filter(|i| passes(i, req, Some(Facet::State))) {
        summary.total += 1;
        match i.status.state {
            SloState::Breached => summary.breached += 1,
            SloState::Warning => summary.warning += 1,
            SloState::Ok => summary.ok += 1,
            SloState::NoData => summary.no_data += 1,
        }
    }
    let mut matched: Vec<SloItem> = items.into_iter().filter(|i| passes(i, req, None)).collect();
    let name = |i: &SloItem| i.name.to_lowercase();
    match req.sort.as_deref() {
        Some("name") => matched.sort_by_key(name),
        Some("sli") => matched.sort_by(|a, b| a.status.sli.unwrap_or(f64::INFINITY).total_cmp(&b.status.sli.unwrap_or(f64::INFINITY)).then_with(|| name(a).cmp(&name(b)))),
        Some("budget") => matched.sort_by(|a, b| a.status.budget_remaining.unwrap_or(f64::INFINITY).total_cmp(&b.status.budget_remaining.unwrap_or(f64::INFINITY)).then_with(|| name(a).cmp(&name(b)))),
        Some("modified") => matched.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
        _ => matched.sort_by(|a, b| b.status.state.severity().cmp(&a.status.state.severity()).then_with(|| name(a).cmp(&name(b)))),
    }
    let total = matched.len() as i64;
    let limit = if req.limit <= 0 { 50 } else { req.limit.min(200) };
    let slos = matched.into_iter().skip(req.offset.max(0) as usize).take(limit as usize).collect();
    SloSearchPage { slos, total, summary, facets }
}

#[utoipa::path(post, path = "/api/v1/slos/search", operation_id = "search_slos", request_body = SloSearchRequest, responses((status = 200, body = SloSearchPage)), tag = "slos")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<SloSearchRequest>) -> ApiResult<SloSearchPage> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE s.org_id = $1")).bind(&org).fetch_all(&st.postgres).await?;
    let corrections = load_corrections(&st, &org, now_ms() - SloWindow::NinetyDays.ms()).await?;
    let (st, org, corrections) = (&st, &org, &corrections);
    let items: Vec<SloItem> = futures::stream::iter(rows)
        .map(|row| async move {
            let window = req.window.unwrap_or(row.primary());
            let (status, _) = status_over(st, org, &row, window, &correction_spans(corrections, &row.id)).await;
            row.item(status)
        })
        .buffered(8)
        .collect()
        .await;
    Ok(Json(search_page(items, &req)))
}

// ================================================================ detail

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SloHistoryPoint {
    /// Bucket start, epoch ms.
    pub t: i64,
    /// SLI of this bucket alone.
    pub bucket_sli: Option<f64>,
    /// SLI from the window's start to the end of this bucket.
    pub sli: Option<f64>,
    /// Error budget left at the end of this bucket, in percent of the window's budget.
    pub budget_remaining: Option<f64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct SloMonitor {
    pub id: i64,
    pub name: String,
    pub status: String,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionCategory {
    ScheduledMaintenance,
    OutsideBusinessHours,
    Deployment,
    Other,
}

impl CorrectionCategory {
    fn as_str(self) -> &'static str {
        match self {
            CorrectionCategory::ScheduledMaintenance => "scheduled_maintenance",
            CorrectionCategory::OutsideBusinessHours => "outside_business_hours",
            CorrectionCategory::Deployment => "deployment",
            CorrectionCategory::Other => "other",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "scheduled_maintenance" => CorrectionCategory::ScheduledMaintenance,
            "outside_business_hours" => CorrectionCategory::OutsideBusinessHours,
            "deployment" => CorrectionCategory::Deployment,
            _ => CorrectionCategory::Other,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SloCorrection {
    pub id: String,
    /// Absent = applies to every SLO of the organization.
    pub slo_id: Option<String>,
    pub slo_name: Option<String>,
    pub category: CorrectionCategory,
    pub description: String,
    pub start: i64,
    pub end: i64,
    pub creator: Option<MemberRef>,
    pub created_at: i64,
}

impl From<CorrectionRow> for SloCorrection {
    fn from(r: CorrectionRow) -> Self {
        SloCorrection {
            id: r.id,
            slo_id: r.slo_id,
            slo_name: r.slo_name,
            category: CorrectionCategory::parse(&r.category),
            description: r.description,
            start: r.start_at,
            end: r.end_at,
            creator: r.creator_id.map(|id| MemberRef { id, name: r.creator_name.unwrap_or_default() }),
            created_at: r.created_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SloDetail {
    /// With its status over the requested window.
    pub slo: SloItem,
    /// Status over each target's window.
    pub statuses: Vec<SloStatus>,
    /// SLI and error budget over the requested window.
    pub history: Vec<SloHistoryPoint>,
    /// Corrections overlapping the requested window (the SLO's own and the org-wide ones).
    pub corrections: Vec<SloCorrection>,
    /// The SLO's monitors and the monitors tagged `slo:<id>` (burn rate alerts).
    pub monitors: Vec<SloMonitor>,
}

#[derive(Deserialize, IntoParams)]
pub struct SloDetailParams {
    /// 1d | 7d | 30d | 90d; absent = the primary window.
    pub window: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/slos/{id}", operation_id = "get_slo", params(("id" = String, Path), SloDetailParams), responses((status = 200, body = SloDetail)), tag = "slos")]
pub async fn get_one(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>, Query(p): Query<SloDetailParams>) -> ApiResult<SloDetail> {
    let row = load_one(&st, &org, &id).await?;
    let window = match p.window.as_deref() {
        None | Some("") | Some("primary") => row.primary(),
        Some(w) => SloWindow::parse(w).ok_or_else(|| ApiError::BadRequest(format!("unknown window '{w}'")))?,
    };
    let now = now_ms();
    let all = load_corrections(&st, &org, now - SloWindow::NinetyDays.ms()).await?;
    let spans = correction_spans(&all, &row.id);
    let (status, measured) = status_over(&st, &org, &row, window, &spans).await;
    let target = target_for(&row.targets(), window, row.primary());
    let history = measured
        .map(|m| math::history(&m.buckets, target.target).into_iter().map(|h| SloHistoryPoint { t: h.t, bucket_sli: h.bucket_sli, sli: h.sli, budget_remaining: h.budget_remaining }).collect())
        .unwrap_or_default();
    let mut statuses = Vec::new();
    for t in row.targets() {
        statuses.push(if t.window == window { status.clone() } else { status_over(&st, &org, &row, t.window, &spans).await.0 });
    }
    let since = now - window.ms();
    let corrections = all.into_iter().filter(|c| c.end_at > since && c.slo_id.as_deref().is_none_or(|s| s == row.id)).map(Into::into).collect();
    let monitors: Vec<SloMonitor> = sqlx::query_as("SELECT id, name, status FROM monitors WHERE org_id = $1 AND (id = ANY($2) OR $3 = ANY(tags)) ORDER BY name")
        .bind(&org)
        .bind(&row.monitor_ids)
        .bind(format!("slo:{}", row.id))
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(SloDetail { slo: row.item(status), statuses, history, corrections, monitors }))
}

// ================================================================ writes

/// Every monitor of the input belongs to the org.
async fn check_monitors(st: &AppState, org: &str, ids: &[i64]) -> Result<(), ApiError> {
    if ids.is_empty() {
        return Ok(());
    }
    let found: Vec<i64> = sqlx::query_scalar("SELECT id FROM monitors WHERE org_id = $1 AND id = ANY($2)").bind(org).bind(ids).fetch_all(&st.postgres).await?;
    match ids.iter().find(|i| !found.contains(i)) {
        Some(missing) => Err(ApiError::BadRequest(format!("Monitor {missing} not found"))),
        None => Ok(()),
    }
}

async fn checked(st: &AppState, org: &str, mut input: SloInput) -> Result<SloInput, ApiError> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    check_monitors(st, org, &input.monitor_ids).await?;
    Ok(input)
}

async fn detail_for(st: &AppState, org: &str, id: &str) -> Result<SloItem, ApiError> {
    let row = load_one(st, org, id).await?;
    let all = load_corrections(st, org, now_ms() - SloWindow::NinetyDays.ms()).await?;
    let (status, _) = status_over(st, org, &row, row.primary(), &correction_spans(&all, &row.id)).await;
    Ok(row.item(status))
}

#[utoipa::path(post, path = "/api/v1/slos", operation_id = "create_slo", request_body = SloInput, responses((status = 200, body = SloItem)), tag = "slos")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<SloInput>) -> ApiResult<SloItem> {
    let input = checked(&st, &ctx.org_id, input).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM slos WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_SLOS {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_SLOS} SLOs")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO slos (org_id, name, description, type, good_query, total_query, monitor_ids, slice_query, comparator, threshold, slice_minutes, targets, primary_window, tags, teams, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.r#type.as_str())
    .bind(&input.good_query)
    .bind(&input.total_query)
    .bind(&input.monitor_ids)
    .bind(&input.slice_query)
    .bind(input.comparator.map(SloComparator::as_str))
    .bind(input.threshold)
    .bind(input.slice_minutes)
    .bind(serde_json::to_value(&input.targets).unwrap_or_default())
    .bind(input.primary_window.map(SloWindow::as_str))
    .bind(&input.tags)
    .bind(&input.teams)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    forget(&ctx.org_id);
    ctx.audit(&st, "slo.create", &format!("slo:{id}"), &input.name).await;
    Ok(Json(detail_for(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/slos/{id}", operation_id = "update_slo", params(("id" = String, Path)), request_body = SloInput, responses((status = 200, body = SloItem)), tag = "slos")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SloInput>) -> ApiResult<SloItem> {
    let input = checked(&st, &ctx.org_id, input).await?;
    let r = sqlx::query(
        "UPDATE slos SET name = $3, description = $4, type = $5, good_query = $6, total_query = $7, monitor_ids = $8, slice_query = $9, comparator = $10, \
         threshold = $11, slice_minutes = $12, targets = $13, primary_window = $14, tags = $15, teams = $16, updated_at = now() WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.r#type.as_str())
    .bind(&input.good_query)
    .bind(&input.total_query)
    .bind(&input.monitor_ids)
    .bind(&input.slice_query)
    .bind(input.comparator.map(SloComparator::as_str))
    .bind(input.threshold)
    .bind(input.slice_minutes)
    .bind(serde_json::to_value(&input.targets).unwrap_or_default())
    .bind(input.primary_window.map(SloWindow::as_str))
    .bind(&input.tags)
    .bind(&input.teams)
    .execute(&st.postgres)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("SLO {id} not found")));
    }
    forget(&ctx.org_id);
    ctx.audit(&st, "slo.update", &format!("slo:{id}"), &input.name).await;
    Ok(Json(detail_for(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/slos/{id}", operation_id = "delete_slo", params(("id" = String, Path)), responses((status = 204)), tag = "slos")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let name: Option<(String,)> = sqlx::query_as("DELETE FROM slos WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let (name,) = name.ok_or_else(|| ApiError::NotFound(format!("SLO {id} not found")))?;
    forget(&ctx.org_id);
    ctx.audit(&st, "slo.delete", &format!("slo:{id}"), &name).await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ corrections

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SloCorrectionInput {
    /// Absent = every SLO of the organization.
    pub slo_id: Option<String>,
    pub category: CorrectionCategory,
    #[serde(default)]
    pub description: String,
    /// Epoch ms.
    pub start: i64,
    pub end: i64,
}

fn validate_correction(input: &mut SloCorrectionInput) -> Result<(), String> {
    input.description = input.description.trim().to_string();
    input.slo_id = input.slo_id.take().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if input.end <= input.start {
        return Err("A correction ends after it starts".into());
    }
    if input.end - input.start > MAX_CORRECTION_DAYS * DAY_MS {
        return Err(format!("A correction covers at most {MAX_CORRECTION_DAYS} days"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    Ok(())
}

async fn checked_correction(st: &AppState, org: &str, mut input: SloCorrectionInput) -> Result<SloCorrectionInput, ApiError> {
    validate_correction(&mut input).map_err(ApiError::BadRequest)?;
    if let Some(slo) = &input.slo_id {
        load_one(st, org, slo).await?;
    }
    Ok(input)
}

async fn correction(st: &AppState, org: &str, id: &str) -> Result<SloCorrection, ApiError> {
    let row: Option<CorrectionRow> = sqlx::query_as(&format!("{CORRECTION_SELECT} WHERE c.org_id = $1 AND c.id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(Into::into).ok_or_else(|| ApiError::NotFound(format!("correction {id} not found")))
}

#[derive(Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct SloCorrectionParams {
    /// Only this SLO's corrections and the org-wide ones.
    pub slo_id: Option<String>,
}

/// The org's corrections, newest first (at most 1,000).
#[utoipa::path(get, path = "/api/v1/slo-corrections", operation_id = "list_slo_corrections", params(SloCorrectionParams), responses((status = 200, body = Vec<SloCorrection>)), tag = "slos")]
pub async fn list_corrections(State(st): State<AppState>, OrgId(org): OrgId, Query(p): Query<SloCorrectionParams>) -> ApiResult<Vec<SloCorrection>> {
    let rows: Vec<CorrectionRow> =
        sqlx::query_as(&format!("{CORRECTION_SELECT} WHERE c.org_id = $1 AND ($2::text IS NULL OR c.slo_id IS NULL OR c.slo_id::text = $2) ORDER BY c.start_at DESC LIMIT 1000"))
            .bind(&org)
            .bind(p.slo_id.filter(|s| !s.is_empty()))
            .fetch_all(&st.postgres)
            .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[utoipa::path(post, path = "/api/v1/slo-corrections", operation_id = "create_slo_correction", request_body = SloCorrectionInput, responses((status = 200, body = SloCorrection)), tag = "slos")]
pub async fn create_correction(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<SloCorrectionInput>) -> ApiResult<SloCorrection> {
    let input = checked_correction(&st, &ctx.org_id, input).await?;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO slo_corrections (org_id, slo_id, category, description, start_at, end_at, created_by) \
         VALUES ($1, $2::uuid, $3, $4, to_timestamp($5), to_timestamp($6), $7::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.slo_id)
    .bind(input.category.as_str())
    .bind(&input.description)
    .bind(input.start as f64 / 1000.0)
    .bind(input.end as f64 / 1000.0)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    forget(&ctx.org_id);
    ctx.audit(&st, "slo_correction.create", &format!("slo_correction:{id}"), &input.description).await;
    Ok(Json(correction(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/slo-corrections/{id}", operation_id = "update_slo_correction", params(("id" = String, Path)), request_body = SloCorrectionInput, responses((status = 200, body = SloCorrection)), tag = "slos")]
pub async fn update_correction(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SloCorrectionInput>) -> ApiResult<SloCorrection> {
    let input = checked_correction(&st, &ctx.org_id, input).await?;
    let r = sqlx::query(
        "UPDATE slo_corrections SET slo_id = $3::uuid, category = $4, description = $5, start_at = to_timestamp($6), end_at = to_timestamp($7) \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.slo_id)
    .bind(input.category.as_str())
    .bind(&input.description)
    .bind(input.start as f64 / 1000.0)
    .bind(input.end as f64 / 1000.0)
    .execute(&st.postgres)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("correction {id} not found")));
    }
    forget(&ctx.org_id);
    ctx.audit(&st, "slo_correction.update", &format!("slo_correction:{id}"), &input.description).await;
    Ok(Json(correction(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/slo-corrections/{id}", operation_id = "delete_slo_correction", params(("id" = String, Path)), responses((status = 204)), tag = "slos")]
pub async fn delete_correction(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let r = sqlx::query("DELETE FROM slo_corrections WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("correction {id} not found")));
    }
    forget(&ctx.org_id);
    ctx.audit(&st, "slo_correction.delete", &format!("slo_correction:{id}"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric_input() -> SloInput {
        SloInput {
            name: "  Checkout availability ".into(),
            description: String::new(),
            r#type: SloType::Metric,
            good_query: Some("sum:trace.http.request.hits{service:order-service,!http.status_code:5*}".into()),
            total_query: Some(" sum:trace.http.request.hits{service:order-service} ".into()),
            monitor_ids: vec![4],
            slice_query: Some("avg:x{*}".into()),
            comparator: None,
            threshold: None,
            slice_minutes: None,
            targets: vec![SloTarget { window: SloWindow::ThirtyDays, target: 99.9, warning: Some(99.95) }, SloTarget { window: SloWindow::SevenDays, target: 99.5, warning: None }],
            primary_window: None,
            tags: vec![" service:order-service".into(), "service:order-service".into(), "".into()],
            teams: vec!["checkout".into()],
        }
    }

    #[test]
    fn validation_trims_and_keeps_only_the_type_fields() {
        let mut i = metric_input();
        validate(&mut i).unwrap();
        assert_eq!(i.name, "Checkout availability");
        assert_eq!(i.total_query.as_deref(), Some("sum:trace.http.request.hits{service:order-service}"));
        assert!(i.monitor_ids.is_empty() && i.slice_query.is_none());
        assert_eq!(i.tags, vec!["service:order-service"]);
        // The first target given is the primary one; targets are sorted by window.
        assert_eq!(i.primary_window, Some(SloWindow::ThirtyDays));
        assert_eq!(i.targets[0].window, SloWindow::SevenDays);
    }

    #[test]
    fn validation_rejects_bad_definitions() {
        let bad = |f: &dyn Fn(&mut SloInput)| {
            let mut i = metric_input();
            f(&mut i);
            validate(&mut i).unwrap_err()
        };
        assert!(bad(&|i| i.name = " ".into()).contains("Name"));
        assert!(bad(&|i| i.good_query = None).contains("good events"));
        assert!(bad(&|i| i.total_query = Some("median:x{*}".into())).contains("Invalid total events"));
        assert!(bad(&|i| i.targets[0].target = 100.0).contains("below 100"));
        assert!(bad(&|i| i.targets[0].warning = Some(99.0)).contains("warning"));
        assert!(bad(&|i| i.targets[1].window = SloWindow::ThirtyDays).contains("one target per"));
        assert!(bad(&|i| i.primary_window = Some(SloWindow::NinetyDays)).contains("primary"));
        assert!(bad(&|i| i.targets.clear()).contains("1–4"));
        assert!(bad(&|i| {
            i.r#type = SloType::Monitor;
            i.monitor_ids.clear();
        })
        .contains("monitors"));
        assert!(bad(&|i| {
            i.r#type = SloType::TimeSlice;
            i.comparator = Some(SloComparator::Lt);
            i.threshold = Some(0.5);
            i.slice_minutes = Some(2);
        })
        .contains("1 or 5"));
    }

    #[test]
    fn a_window_without_its_own_target_uses_the_primary_one() {
        let targets = [SloTarget { window: SloWindow::SevenDays, target: 99.5, warning: None }, SloTarget { window: SloWindow::ThirtyDays, target: 99.9, warning: Some(99.95) }];
        assert_eq!(target_for(&targets, SloWindow::SevenDays, SloWindow::ThirtyDays).target, 99.5);
        let day = target_for(&targets, SloWindow::OneDay, SloWindow::ThirtyDays);
        assert_eq!((day.window, day.target, day.warning), (SloWindow::OneDay, 99.9, Some(99.95)));
    }

    #[test]
    fn status_reports_budget_in_events_or_minutes() {
        let target = SloTarget { window: SloWindow::SevenDays, target: 99.0, warning: Some(99.5) };
        let events = Measured { buckets: vec![(0, Measure { good: 9_920.0, total: 10_000.0 })], last_hour: Measure { good: 90.0, total: 100.0 }, unit: BudgetUnit::Events };
        let s = status(&events, target);
        assert_eq!(s.state, SloState::Warning);
        assert_eq!(s.sli, Some(99.2));
        assert!((s.budget_allowed.unwrap() - 100.0).abs() < 1e-6);
        assert!((s.budget_used.unwrap() - 80.0).abs() < 1e-6);
        assert!((s.budget_remaining.unwrap() - 20.0).abs() < 1e-6);
        assert!((s.burn_rate_1h.unwrap() - 10.0).abs() < 1e-6);
        // A day of uptime with 30 minutes down against 99%: 14.4 minutes allowed.
        let minutes = Measured { buckets: vec![(0, Measure { good: (DAY_MS - 30 * MIN_MS) as f64, total: DAY_MS as f64 })], last_hour: Measure::default(), unit: BudgetUnit::Minutes };
        let s = status(&minutes, target);
        assert_eq!(s.state, SloState::Breached);
        assert!((s.total - 1440.0).abs() < 1e-6);
        assert!((s.budget_allowed.unwrap() - 14.4).abs() < 1e-6);
        assert!((s.budget_used.unwrap() - 30.0).abs() < 1e-6);
        assert_eq!(s.burn_rate_1h, None);
        assert_eq!(no_data(target, SloType::Monitor).state, SloState::NoData);
    }

    fn item(name: &str, state: SloState, tags: &[&str], burn: Option<f64>) -> SloItem {
        let mut status = no_data(SloTarget { window: SloWindow::ThirtyDays, target: 99.0, warning: None }, SloType::Metric);
        status.state = state;
        status.burn_rate_1h = burn;
        Row {
            id: name.into(),
            name: name.into(),
            description: String::new(),
            kind: "metric".into(),
            good_query: None,
            total_query: None,
            monitor_ids: vec![],
            slice_query: None,
            comparator: None,
            threshold: None,
            slice_minutes: None,
            targets: Value::Null,
            primary_window: "30d".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            teams: vec![],
            creator_id: None,
            creator_name: None,
            created_at: 0,
            updated_at: 0,
        }
        .item(status)
    }

    #[test]
    fn search_counts_states_facets_and_filters() {
        let items = vec![
            item("Billing latency", SloState::Breached, &["service:billing", "env:prod"], Some(12.0)),
            item("Checkout availability", SloState::Warning, &["service:order-service", "env:prod"], Some(2.0)),
            item("Catalog availability", SloState::Ok, &["service:catalog-service", "env:staging", "tier:1"], Some(0.1)),
        ];
        let req = SloSearchRequest { states: vec![SloState::Ok], ..Default::default() };
        let page = search_page(items.clone(), &req);
        assert_eq!(page.total, 1);
        // The summary and the state facet ignore the state filter.
        assert_eq!((page.summary.total, page.summary.breached, page.summary.warning, page.summary.ok), (3, 1, 1, 1));
        assert_eq!(page.facets.state.len(), 3);
        // Other facets count the state-filtered SLOs.
        assert_eq!(page.facets.envs.iter().map(|f| f.value.as_str()).collect::<Vec<_>>(), vec!["staging"]);

        let page = search_page(items.clone(), &SloSearchRequest { query: "availability env:prod".into(), ..Default::default() });
        assert_eq!(page.slos.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Checkout availability"]);

        // Worst state first.
        let page = search_page(items.clone(), &SloSearchRequest::default());
        assert_eq!(page.slos[0].name, "Billing latency");
        assert_eq!(page.facets.burn_rate.iter().find(|f| f.value == "high").map(|f| f.count), Some(1));
        assert_eq!(page.facets.tags.iter().map(|f| f.value.as_str()).collect::<Vec<_>>(), vec!["tier:1"]);
        let page = search_page(items, &SloSearchRequest { burn_rates: vec![BurnLevel::Elevated], services: vec!["order-service".into()], ..Default::default() });
        assert_eq!(page.total, 1);
    }

    #[test]
    fn corrections_apply_to_their_slo_and_org_wide() {
        let c = |slo: Option<&str>, start: i64, end: i64| CorrectionRow {
            id: "c".into(),
            slo_id: slo.map(Into::into),
            slo_name: None,
            category: "deployment".into(),
            description: String::new(),
            start_at: start,
            end_at: end,
            creator_id: None,
            creator_name: None,
            created_at: 0,
        };
        let all = vec![c(Some("a"), 0, 10), c(None, 5, 20), c(Some("b"), 30, 40)];
        assert_eq!(correction_spans(&all, "a"), vec![(0, 20)]);
        assert_eq!(correction_spans(&all, "b"), vec![(5, 20), (30, 40)]);
        let mut input = SloCorrectionInput { slo_id: Some(" ".into()), category: CorrectionCategory::Other, description: " x ".into(), start: 10, end: 10 };
        assert!(validate_correction(&mut input).is_err());
        input.end = 11;
        validate_correction(&mut input).unwrap();
        assert_eq!((input.slo_id, input.description.as_str()), (None, "x"));
    }
}
