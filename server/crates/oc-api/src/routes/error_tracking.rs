//! Error Tracking: error logs grouped into issues by service, error type and message pattern.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{parse_query, Param};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::routes::logs::{bind, scope, TimeRange};
use crate::state::{AppState, OrgId};

/// Error logs of one issue key (service, kind, [`shape_key`]), aggregated by ClickHouse.
#[derive(clickhouse::Row, Deserialize, Debug, Clone)]
pub(crate) struct ErrorRow {
    pub service: String,
    /// `error.kind` / `error.type` / `exception.type` when the logs have one.
    pub kind: String,
    pub key: String,
    /// The newest message (first 300 characters). Selected as `sample`: an alias named `message`
    /// would shadow the column in GROUP BY.
    #[serde(rename = "sample")]
    pub message: String,
    pub n: u64,
    /// Unix ms of the first and last log.
    pub first: i64,
    pub last: i64,
    /// Sparkline buckets with logs, and their counts.
    pub spark: (Vec<u32>, Vec<u64>),
}

/// The error's type: the log's error kind attribute, else the first `…Error` / `…Exception`
/// word of its message (`redis.exceptions.AuthenticationError`, `ValueError`), else `Error`.
pub(crate) fn error_type(message: &str, kind: &str) -> String {
    if !kind.is_empty() {
        return kind.to_string();
    }
    message
        .split_whitespace()
        .map(|w| w.trim_end_matches(|c: char| !c.is_alphanumeric()))
        .find(|w| {
            w.len() > 5
                && w.starts_with(|c: char| c.is_ascii_alphabetic())
                && (w.ends_with("Error") || w.ends_with("Exception"))
                && w.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '$')
        })
        .unwrap_or("Error")
        .to_string()
}

/// A stable issue id (FNV-1a 64, hex): the same group gets the same id across restarts and versions.
pub(crate) fn issue_id(service: &str, kind: &str, key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [service, kind, key].join("\u{1f}").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Debug)]
pub(crate) struct IssueGroup {
    pub id: String,
    pub service: String,
    pub kind: String,
    pub key: String,
    pub error_type: String,
    /// The newest message of the group.
    pub message: String,
    pub count: u64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub sparkline: Vec<u64>,
}

/// ClickHouse's groups as issues, most frequent first.
pub(crate) fn group_issues(rows: Vec<ErrorRow>, buckets: usize) -> Vec<IssueGroup> {
    let mut out: Vec<IssueGroup> = rows
        .into_iter()
        .map(|r| {
            let first_line = strip_ansi(r.message.lines().next().unwrap_or(""));
            let mut sparkline = vec![0; buckets];
            for (b, n) in r.spark.0.iter().zip(&r.spark.1) {
                sparkline[(*b as usize).min(buckets - 1)] += n;
            }
            IssueGroup {
                id: issue_id(&r.service, &r.kind, &r.key),
                error_type: error_type(&first_line, &r.kind),
                service: r.service,
                kind: r.kind,
                key: r.key,
                message: first_line,
                count: r.n,
                first_seen: r.first,
                last_seen: r.last,
                sparkline,
            }
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
    out
}

/// The text without terminal color codes (`ESC [ … m` and other CSI sequences).
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            // Parameters and intermediates, up to the final byte (@ to ~).
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The runtime, when the error's type or message shows it; `unknown` rather than a guess.
pub(crate) fn platform_of(error_type: &str, message: &str) -> &'static str {
    if error_type.starts_with("java.") || error_type.starts_with("javax.") {
        "java"
    } else if error_type.starts_with("System.") || error_type.starts_with("Microsoft.") {
        "dotnet"
    } else if message.contains("Traceback (most recent call last)") || message.contains("File \"") {
        "python"
    } else if message.starts_with("panic:") || message.contains("goroutine ") {
        "go"
    } else {
        "unknown"
    }
}

/// Whether a log (its service, error kind and message) belongs to the issue `id`.
pub(crate) fn log_matches(id: &str, service: &str, kind: &str, message: &str) -> bool {
    issue_id(service, kind, &shape_key(message)) == id
}

/// A message's shape, as ClickHouse computes it for `error_key`: its first 200 characters with each
/// run of hex characters containing a digit (numbers, ids, hashes, timestamps' parts) as `0`.
pub(crate) fn shape(message: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.chars().any(|c| c.is_ascii_digit()) {
            out.push('0');
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in message.chars().take(200) {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// The SQL of [`shape`].
#[cfg(test)]
const SHAPE_SQL: &str = "replaceRegexpAll(substringUTF8(message, 1, 200), '[0-9A-Fa-f]*[0-9][0-9A-Fa-f]*', '0')";

/// The issue key's message part: the first 80 characters of its shape. Long enough to tell errors
/// apart, short enough that variable tails (paths, quoted values) don't make one issue per log.
pub(crate) fn shape_key(message: &str) -> String {
    shape(message).chars().take(80).collect()
}

/// The SQL of [`shape_key`] (oc.logs' `error_key` column).
#[cfg(test)]
fn key_sql() -> String {
    format!("substringUTF8({SHAPE_SQL}, 1, 80)")
}

/// The SQL of the error kind attribute (oc.logs' `error_kind` column).
#[cfg(test)]
const KIND_SQL: &str = "multiIf(attr['error.kind'] != '', attr['error.kind'], attr['error.type'] != '', attr['error.type'], attr['exception.type'])";

#[derive(Serialize, ToSchema, Debug)]
pub struct TagShare {
    pub key: String,
    pub value: String,
    /// 0..1 of the issue's logs.
    pub share: f64,
}

/// Each value's share of the logs, per key (top 5 values per key).
pub(crate) fn tag_distribution(logs: &[HashMap<String, String>], keys: &[&str]) -> Vec<TagShare> {
    let total = logs.len().max(1) as f64;
    let mut out = Vec::new();
    for key in keys {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for l in logs {
            if let Some(v) = l.get(*key).filter(|v| !v.is_empty()) {
                *counts.entry(v.as_str()).or_default() += 1;
            }
        }
        let mut values: Vec<(&str, usize)> = counts.into_iter().collect();
        values.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        out.extend(values.into_iter().take(5).map(|(v, n)| TagShare { key: key.to_string(), value: v.to_string(), share: n as f64 / total }));
    }
    out
}

/// A user an issue is assigned to.
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct UserRef {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// A saved decision (Postgres `error_issue_states`).
pub(crate) struct Saved {
    pub status: String,
    /// Unix ms of the last status change.
    pub updated_at: i64,
    pub assignee: Option<UserRef>,
}

const STATES: [&str; 4] = ["for_review", "reviewed", "ignored", "resolved"];

/// An issue with its status, whether it regressed, and its assignee.
pub(crate) struct StatedIssue {
    pub group: IssueGroup,
    pub status: String,
    pub regression: bool,
    pub assignee: Option<UserRef>,
}

/// Issues with their status (`for_review` unless saved) and whether they regressed (resolved, then
/// seen again after the resolution), filtered to `state` (or `all`), and the totals per state.
pub(crate) fn apply_states(issues: Vec<IssueGroup>, saved: &HashMap<String, Saved>, state: &str) -> (Vec<StatedIssue>, HashMap<String, usize>) {
    let mut totals: HashMap<String, usize> = STATES.iter().map(|s| (s.to_string(), 0)).collect();
    totals.insert("all".into(), issues.len());
    let mut shown = Vec::new();
    for g in issues {
        let saved = saved.get(&g.id);
        let (status, regression) = match saved {
            Some(s) if s.status == "resolved" && g.last_seen > s.updated_at => ("for_review".to_string(), true),
            Some(s) => (s.status.clone(), false),
            None => ("for_review".to_string(), false),
        };
        *totals.entry(status.clone()).or_default() += 1;
        if state == "all" || state == status {
            shown.push(StatedIssue { group: g, status, regression, assignee: saved.and_then(|s| s.assignee.clone()) });
        }
    }
    (shown, totals)
}

/// Issue keys read per search.
const GROUP_LIMIT: i64 = 50_000;
/// Sparkline buckets per issue.
const SPARK_BUCKETS: usize = 12;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueSearchRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// for_review | reviewed | ignored | resolved | all
    #[serde(default = "all_state")]
    pub state: String,
    /// logs | rum | apm | all
    #[serde(default = "all_state")]
    pub source: String,
    /// relevance | count | last_seen | first_seen
    #[serde(default)]
    pub sort: Option<String>,
    /// Offset into the result, as returned in `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    pub limit: usize,
}

fn all_state() -> String {
    "all".into()
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueOut {
    pub id: String,
    pub error_type: String,
    pub message: String,
    pub platform: String,
    pub persona: String,
    pub source: String,
    pub service: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub count: u64,
    pub sparkline: Vec<u64>,
    pub status: String,
    pub assignee: Option<UserRef>,
    pub team: Option<String>,
    pub is_crash: bool,
    pub is_regression: bool,
    pub suspected_cause: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueSearchPage {
    pub issues: Vec<IssueOut>,
    pub next_cursor: Option<String>,
    /// Issues per state tab (for_review, reviewed, ignored, resolved, all).
    pub totals: HashMap<String, usize>,
}

/// The organization's issues from error logs in `range` (matching `query`), `buckets` per sparkline;
/// with `only`, just that issue's logs.
async fn load_issues(st: &AppState, org: &str, range: TimeRange, query: &str, buckets: usize, only: Option<&IssueGroup>) -> Result<Vec<IssueGroup>, ApiError> {
    let mut terms = parse_query(query);
    if let Some(g) = only {
        terms.push(oc_query::Term::Tag { key: "service".into(), values: vec![g.service.clone()], negate: false });
    }
    let s = scope(org, range, &terms, None);
    let width = ((range.to - range.from) / buckets as i64).max(1);
    // Bucket parameters first (they come first in the SQL), then the scope.
    let mut params = vec![Param::Int(range.from), Param::Int(width)];
    params.extend(s.params);
    let narrowing = match only {
        Some(g) => {
            params.push(Param::Str(g.kind.clone()));
            params.push(Param::Str(g.key.clone()));
            " AND error_kind = ? AND error_key = ?".to_string()
        }
        None => String::new(),
    };
    params.push(Param::Int(GROUP_LIMIT));
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, \
             argMax(substringUTF8(message, 1, 300), timestamp) AS sample, count() AS n, \
             toUnixTimestamp64Milli(min(timestamp)) AS first, toUnixTimestamp64Milli(max(timestamp)) AS last, \
             sumMap([toUInt32(intDiv(toUnixTimestamp64Milli(timestamp) - ?, ?))], [toUInt64(1)]) AS spark \
             FROM {} WHERE {} AND status = 'error'{narrowing} \
             GROUP BY service, error_kind, error_key ORDER BY n DESC LIMIT ?",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all::<ErrorRow>()
    .await?;
    Ok(group_issues(rows, buckets))
}

/// Saved decisions of the organization's issues.
async fn saved_states(st: &AppState, org: &str) -> Result<HashMap<String, Saved>, ApiError> {
    type SavedRow = (String, String, i64, Option<String>, Option<String>, Option<String>);
    Ok(sqlx::query_as::<_, SavedRow>(
        "SELECT s.issue_id, s.status, (extract(epoch FROM s.status_at) * 1000)::bigint, u.id::text, u.name, u.email \
         FROM error_issue_states s LEFT JOIN users u ON u.id = s.assignee AND u.org_id = s.org_id WHERE s.org_id = $1",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?
    .into_iter()
    .map(|(id, status, updated_at, uid, name, email)| {
        let assignee = uid.map(|id| UserRef { id, name: name.unwrap_or_default(), email: email.unwrap_or_default() });
        (id, Saved { status, updated_at, assignee })
    })
    .collect())
}

fn issue_out(i: StatedIssue) -> IssueOut {
    let StatedIssue { group: g, status, regression, assignee } = i;
    IssueOut {
        platform: platform_of(&g.error_type, &g.message).into(),
        persona: "backend".into(),
        source: "logs".into(),
        status,
        assignee,
        team: None,
        is_crash: false,
        is_regression: regression,
        suspected_cause: None,
        id: g.id,
        error_type: g.error_type,
        message: g.message,
        service: g.service,
        first_seen: g.first_seen,
        last_seen: g.last_seen,
        count: g.count,
        sparkline: g.sparkline,
    }
}

/// Issues from the organization's error logs in the range (first and last seen within it).
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/search", request_body = IssueSearchRequest,
    responses((status = 200, body = IssueSearchPage)), tag = "error-tracking")]
pub async fn search_issues(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<IssueSearchRequest>) -> ApiResult<IssueSearchPage> {
    req.range.validate()?;
    let mut issues = if matches!(req.source.as_str(), "logs" | "all") { load_issues(&st, &org, req.range, &req.query, SPARK_BUCKETS, None).await? } else { Vec::new() };
    match req.sort.as_deref() {
        Some("last_seen") => issues.sort_by(|a, b| b.last_seen.cmp(&a.last_seen)),
        Some("first_seen") => issues.sort_by(|a, b| b.first_seen.cmp(&a.first_seen)),
        _ => {}
    }
    let saved = saved_states(&st, &org).await?;
    let (issues, totals) = apply_states(issues, &saved, &req.state);
    let offset = req.cursor.as_deref().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
    let limit = req.limit.clamp(1, 200);
    let next_cursor = (offset + limit < issues.len()).then(|| (offset + limit).to_string());
    let page = issues.into_iter().skip(offset).take(limit).map(issue_out).collect();
    Ok(Json(IssueSearchPage { issues: page, next_cursor, totals }))
}

#[derive(Deserialize, ToSchema)]
pub struct IssueDetailRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema)]
pub struct HistogramPoint {
    pub t: i64,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetailOut {
    #[serde(flatten)]
    pub issue: IssueOut,
    pub histogram: Vec<HistogramPoint>,
    /// Sessions and users affected (RUM only).
    pub impact: Option<serde_json::Value>,
    pub tags: Vec<TagShare>,
    pub activity: Vec<serde_json::Value>,
}

/// Points of an issue's histogram on the detail page.
const DETAIL_BUCKETS: usize = 30;
/// Logs of an issue read for its tags and samples (the newest or oldest).
const ISSUE_LOGS: i64 = 2_000;
const TAG_KEYS: [&str; 6] = ["host", "env", "version", "kube_namespace", "pod_name", "container_name"];

#[derive(clickhouse::Row, Deserialize)]
struct LogRow {
    id: String,
    t: i64,
    service: String,
    kind: String,
    message: String,
    tags_json: String,
    attributes: String,
}

/// The newest (or oldest) error logs in range that belong to the issue (its service and message shape).
async fn issue_logs(st: &AppState, org: &str, range: TimeRange, query: &str, group: &IssueGroup, oldest: bool) -> Result<Vec<LogRow>, ApiError> {
    let mut terms = parse_query(query);
    terms.push(oc_query::Term::Tag { key: "service".into(), values: vec![group.service.clone()], negate: false });
    let s = scope(org, range, &terms, None);
    let mut params = s.params;
    params.push(Param::Str(group.kind.clone()));
    params.push(Param::Str(group.key.clone()));
    params.push(Param::Int(ISSUE_LOGS));
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(id) AS id, toUnixTimestamp64Milli(timestamp) AS t, toString(service) AS service, \
             toString(error_kind) AS kind, \
             substringUTF8(message, 1, 5000) AS message, toJSONString(mapUpdate(tags, map('host', toString(host)))) AS tags_json, attributes \
             FROM {} WHERE {} AND status = 'error' AND error_kind = ? AND error_key = ? \
             ORDER BY timestamp {} LIMIT ?",
            s.table,
            s.clause,
            if oldest { "ASC" } else { "DESC" }
        )),
        &params,
    )
    .fetch_all::<LogRow>()
    .await?;
    Ok(rows.into_iter().filter(|r| log_matches(&group.id, &r.service, &r.kind, &r.message)).collect())
}

fn valid_issue_id(id: &str) -> Result<(), ApiError> {
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("invalid issue id".into()));
    }
    Ok(())
}

/// One issue in the range: its histogram, the tags of its logs and its saved state.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/{id}", params(("id" = String, Path)), request_body = IssueDetailRequest, responses((status = 200, body = IssueDetailOut)), tag = "error-tracking")]
pub async fn issue_detail(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>, Json(req): Json<IssueDetailRequest>) -> ApiResult<IssueDetailOut> {
    valid_issue_id(&id)?;
    req.range.validate()?;
    let found = load_issues(&st, &org, req.range, &req.query, 1, None).await?.into_iter().find(|g| g.id == id).ok_or_else(|| ApiError::NotFound("no such issue in this time range".into()))?;
    // Its own logs only, for exact counts and a finer histogram.
    let group = load_issues(&st, &org, req.range, &req.query, DETAIL_BUCKETS, Some(&found)).await?.into_iter().find(|g| g.id == id).unwrap_or(found);
    let width = ((req.range.to - req.range.from) / DETAIL_BUCKETS as i64).max(1);
    let histogram = group.sparkline.iter().enumerate().map(|(i, n)| HistogramPoint { t: req.range.from + i as i64 * width, count: *n }).collect();
    let logs = issue_logs(&st, &org, req.range, &req.query, &group, false).await?;
    let tag_maps: Vec<HashMap<String, String>> = logs.iter().map(|l| serde_json::from_str(&l.tags_json).unwrap_or_default()).collect();
    let tags = tag_distribution(&tag_maps, &TAG_KEYS);
    let saved = saved_states(&st, &org).await?;
    let (mut stated, _) = apply_states(vec![group], &saved, "all");
    let issue = issue_out(stated.remove(0));
    Ok(Json(IssueDetailOut { issue, histogram, impact: None, tags, activity: Vec::new() }))
}

#[derive(Deserialize, ToSchema)]
pub struct SampleRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// Position within `order`.
    pub index: usize,
    /// latest | oldest
    #[serde(default)]
    pub order: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct StackOut {
    pub title: String,
    /// Parsed frames (none yet: `raw` holds the message and stack as logged).
    pub frames: Vec<serde_json::Value>,
    pub raw: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SampleOut {
    pub id: String,
    pub timestamp: i64,
    pub tags: HashMap<String, String>,
    pub stack: StackOut,
    pub timeline: Vec<serde_json::Value>,
    pub attribute_groups: Vec<serde_json::Value>,
    pub json: serde_json::Value,
}

#[derive(Serialize, ToSchema)]
pub struct SamplePage {
    pub sample: Option<SampleOut>,
    pub index: usize,
    pub total: usize,
}

/// One log of the issue (the `index`-th newest or oldest in range), and how many there are.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/{id}/samples", params(("id" = String, Path)), request_body = SampleRequest, responses((status = 200, body = SamplePage)), tag = "error-tracking")]
pub async fn issue_samples(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>, Json(req): Json<SampleRequest>) -> ApiResult<SamplePage> {
    valid_issue_id(&id)?;
    req.range.validate()?;
    let Some(group) = load_issues(&st, &org, req.range, &req.query, 1, None).await?.into_iter().find(|g| g.id == id) else {
        return Ok(Json(SamplePage { sample: None, index: 0, total: 0 }));
    };
    let logs = issue_logs(&st, &org, req.range, &req.query, &group, req.order.as_deref() == Some("oldest")).await?;
    let total = logs.len();
    let index = req.index.min(total.saturating_sub(1));
    let sample = logs.into_iter().nth(index).map(|l| {
        let mut json: serde_json::Value = serde_json::from_str(&l.attributes).unwrap_or(serde_json::json!({}));
        if let Some(o) = json.as_object_mut() {
            o.insert("message".into(), serde_json::Value::String(l.message.clone()));
        }
        SampleOut {
            id: l.id,
            timestamp: l.t,
            tags: serde_json::from_str(&l.tags_json).unwrap_or_default(),
            stack: {
                let raw = strip_ansi(&l.message);
                StackOut { title: format!("{}: {}", group.error_type, raw.lines().next().unwrap_or("")), frames: Vec::new(), raw }
            },
            timeline: Vec::new(),
            attribute_groups: Vec::new(),
            json,
        }
    });
    Ok(Json(SamplePage { sample, index, total }))
}

/// `null` → `Some(None)` (unassign), absent → `None` (leave as is).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueUpdate {
    /// for_review, reviewed, ignored or resolved; absent leaves it as is.
    #[serde(default)]
    pub status: Option<String>,
    /// A user of the organization, `null` to unassign; absent leaves it as is.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub assignee_id: Option<Option<String>>,
}

/// Sets an issue's status (reviewed, ignored, resolved, back for review) and/or its assignee.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/{id}/state", params(("id" = String, Path)), request_body = IssueUpdate, responses((status = 204)), tag = "error-tracking")]
pub async fn set_issue_state(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(u): Json<IssueUpdate>) -> Result<StatusCode, ApiError> {
    if let Some(status) = &u.status {
        if !STATES.contains(&status.as_str()) {
            return Err(ApiError::BadRequest(format!("unknown status {status}")));
        }
    }
    valid_issue_id(&id)?;
    // An assignee must be a user of this organization.
    if let Some(Some(user)) = &u.assignee_id {
        let known: Option<(String,)> = sqlx::query_as("SELECT id::text FROM users WHERE id::text = $1 AND org_id = $2").bind(user).bind(&ctx.org_id).fetch_optional(&st.postgres).await?;
        if known.is_none() {
            return Err(ApiError::BadRequest("unknown assignee".into()));
        }
    }
    let set_assignee = u.assignee_id.is_some();
    sqlx::query(
        "INSERT INTO error_issue_states (org_id, issue_id, status, status_at, assignee, updated_by, updated_at) \
         VALUES ($1, $2, COALESCE($3, 'for_review'), now(), $4::uuid, $5::uuid, now()) \
         ON CONFLICT (org_id, issue_id) DO UPDATE SET \
           status = COALESCE($3, error_issue_states.status), \
           status_at = CASE WHEN $3 IS NULL OR $3 = error_issue_states.status THEN error_issue_states.status_at ELSE now() END, \
           assignee = CASE WHEN $6 THEN $4::uuid ELSE error_issue_states.assignee END, \
           updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&u.status)
    .bind(u.assignee_id.flatten())
    .bind(&ctx.user_id)
    .bind(set_assignee)
    .execute(&st.postgres)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(service: &str, message: &str, n: u64, first: i64, last: i64, spark: (Vec<u32>, Vec<u64>)) -> ErrorRow {
        ErrorRow { service: service.into(), kind: String::new(), key: shape_key(message), message: message.into(), n, first, last, spark }
    }

    #[test]
    fn the_error_type_comes_from_the_attributes_or_the_message() {
        assert_eq!(error_type("boom", "java.lang.IllegalStateException"), "java.lang.IllegalStateException");
        assert_eq!(error_type("Traceback ... ValueError: invalid literal for int()", ""), "ValueError");
        assert_eq!(error_type("redis.exceptions.AuthenticationError: Authentication required.", ""), "redis.exceptions.AuthenticationError");
        assert_eq!(error_type("connection refused", ""), "Error");
    }

    #[test]
    fn the_platform_is_named_only_when_the_error_shows_it() {
        assert_eq!(platform_of("java.lang.NullPointerException", ""), "java");
        assert_eq!(platform_of("System.TimeoutException", ""), "dotnet");
        assert_eq!(platform_of("ValueError", "Traceback (most recent call last):"), "python");
        assert_eq!(platform_of("Error", "panic: runtime error: index out of range"), "go");
        assert_eq!(platform_of("Error", "connection refused"), "unknown");
    }

    fn group(id: &str, last_seen: i64) -> IssueGroup {
        IssueGroup {
            id: id.into(),
            service: "billing".into(),
            kind: String::new(),
            key: String::new(),
            error_type: "Error".into(),
            message: String::new(),
            count: 1,
            first_seen: 0,
            last_seen,
            sparkline: vec![],
        }
    }

    #[test]
    fn a_log_belongs_to_the_issue_with_its_service_kind_and_shape_key() {
        let id = issue_id("billing", "", &shape_key("ValueError: order 41 not found"));
        assert!(log_matches(&id, "billing", "", "ValueError: order 982 not found"));
        assert!(!log_matches(&id, "checkout", "", "ValueError: order 982 not found"));
        assert!(!log_matches(&id, "billing", "", "TimeoutError: order 982 not found"));
        assert!(!log_matches(&id, "billing", "java.lang.IllegalStateException", "ValueError: order 982 not found"));
    }

    #[test]
    fn the_shape_matches_the_normalized_message_in_clickhouse() {
        // Same normalization as SHAPE_SQL: runs of hex characters with a digit become 0.
        assert_eq!(shape("2026-10-27T14:20:55.614Z warn internal/metrics_model.go:156 DataPoint"), "0-0-0T0:0:0.0Z warn internal/metrics_model.go:0 DataPoint");
        assert_eq!(shape("order 41 not found"), "order 0 not found");
        // Hashes and ids collapse (so their messages don't make one shape each); words without digits stay.
        assert_eq!(shape("pod 7d9f8c6b5 deleted after 3f2a9c1e-77b4 failed"), "pod 0 deleted after 0-0 failed");
        // The issue key: the shape's first 80 characters.
        assert_eq!(shape_key(&format!("{} 12", "x".repeat(100))), "x".repeat(80));
    }

    #[test]
    fn the_logs_error_columns_compute_the_same_key() {
        // oc.logs' materialized error_kind / error_key columns are this module's SQL.
        let migration = include_str!("../../../../migrations/clickhouse/0007_log_error_keys.sql");
        assert!(migration.contains(&format!("MATERIALIZED if(status = 'error', {}, '')", key_sql())));
        assert!(migration.contains(&format!("MATERIALIZED if(status = 'error', {KIND_SQL}, '')")));
    }

    #[test]
    fn terminal_colors_are_dropped_from_shown_messages() {
        assert_eq!(strip_ansi("Query executed in \u{1b}[92m0.0029s\u{1b}[0m | done"), "Query executed in 0.0029s | done");
        assert_eq!(strip_ansi("\u{1b}[1;31mERROR\u{1b}[0m x"), "ERROR x");
        assert_eq!(strip_ansi("plain [92m text"), "plain [92m text");
        let issues = group_issues(vec![row("billing", "\u{1b}[31mValueError\u{1b}[0m: bad", 1, 0, 0, (vec![0], vec![1]))], 1);
        assert_eq!(issues[0].message, "ValueError: bad");
    }

    #[test]
    fn tag_distribution_gives_each_values_share_of_the_logs() {
        let logs = vec![
            HashMap::from([("host".to_string(), "node-1".to_string()), ("env".to_string(), "prod".to_string())]),
            HashMap::from([("host".to_string(), "node-1".to_string()), ("env".to_string(), "prod".to_string())]),
            HashMap::from([("host".to_string(), "node-2".to_string())]),
        ];
        let d = tag_distribution(&logs, &["host", "env"]);
        let share = |k: &str, v: &str| d.iter().find(|t| t.key == k && t.value == v).map(|t| (t.share * 100.0).round());
        assert_eq!(share("host", "node-1"), Some(67.0));
        assert_eq!(share("host", "node-2"), Some(33.0));
        assert_eq!(share("env", "prod"), Some(67.0));
    }

    #[test]
    fn an_update_tells_a_missing_assignee_from_unassigning() {
        let u: IssueUpdate = serde_json::from_str(r#"{"status":"reviewed"}"#).unwrap();
        assert_eq!((u.status.as_deref(), u.assignee_id), (Some("reviewed"), None));
        let u: IssueUpdate = serde_json::from_str(r#"{"assigneeId":null}"#).unwrap();
        assert_eq!(u.assignee_id, Some(None));
        let u: IssueUpdate = serde_json::from_str(r#"{"assigneeId":"0c4f1d2e-0000-4000-8000-000000000001"}"#).unwrap();
        assert_eq!(u.assignee_id, Some(Some("0c4f1d2e-0000-4000-8000-000000000001".into())));
    }

    #[test]
    fn saved_states_apply_and_a_resolved_issue_that_comes_back_regresses() {
        let saved = HashMap::from([
            ("a".to_string(), Saved { status: "reviewed".into(), updated_at: 500, assignee: Some(UserRef { id: "u1".into(), name: "Ana".into(), email: "ana@example.com".into() }) }),
            ("b".to_string(), Saved { status: "resolved".into(), updated_at: 500, assignee: None }),
            ("c".to_string(), Saved { status: "resolved".into(), updated_at: 500, assignee: None }),
        ]);
        // a: reviewed; b: resolved, not seen since; c: resolved, seen again after → for review, regression; d: unknown → for review.
        let issues = vec![group("a", 900), group("b", 400), group("c", 900), group("d", 900)];
        let (shown, totals) = apply_states(issues, &saved, "all");
        let state = |id: &str| shown.iter().find(|i| i.group.id == id).map(|i| (i.status.as_str(), i.regression)).unwrap();
        assert_eq!(shown.iter().find(|i| i.group.id == "a").unwrap().assignee.as_ref().map(|u| u.name.as_str()), Some("Ana"));
        assert_eq!(state("a"), ("reviewed", false));
        assert_eq!(state("b"), ("resolved", false));
        assert_eq!(state("c"), ("for_review", true));
        assert_eq!(state("d"), ("for_review", false));
        assert_eq!((totals["for_review"], totals["reviewed"], totals["resolved"], totals["ignored"], totals["all"]), (2, 1, 1, 0, 4));
        let (resolved, _) = apply_states(vec![group("b", 400), group("d", 900)], &saved, "resolved");
        assert_eq!(resolved.iter().map(|i| i.group.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
    }

    #[test]
    fn each_clickhouse_key_is_one_issue_with_its_sparkline() {
        // One row per (service, kind, shape key), aggregated by ClickHouse with its buckets.
        let rows = vec![
            row("billing", "ValueError: order 41 not found", 5, 1_000, 2_000, (vec![0, 1], vec![3, 2])),
            row("billing", "TimeoutError: upstream timed out after 30s", 1, 3_000, 3_000, (vec![2], vec![1])),
            row("checkout", "ValueError: order 7 not found", 1, 4_000, 4_000, (vec![3], vec![1])),
        ];
        let issues = group_issues(rows, 12);
        assert_eq!(issues.len(), 3);
        let top = &issues[0];
        assert_eq!((top.service.as_str(), top.error_type.as_str(), top.count), ("billing", "ValueError", 5));
        assert_eq!((top.first_seen, top.last_seen), (1_000, 2_000));
        assert_eq!(&top.sparkline[..3], &[3, 2, 0]);
        assert_eq!(top.sparkline.len(), 12);
        // Stable ids from the key, the same one ClickHouse groups by.
        assert_eq!(top.id, issue_id("billing", "", &shape_key("ValueError: order 982 not found")));
    }
}
