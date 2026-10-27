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
use crate::routes::logs::{bind, pattern_of, scope, TimeRange};
use crate::state::{AppState, OrgId};

/// Error logs of one (service, kind, digit-normalized message, time bucket), aggregated by ClickHouse.
#[derive(clickhouse::Row, Deserialize, Debug, Clone)]
pub(crate) struct ErrorRow {
    pub service: String,
    /// `error.kind` / `error.type` / `exception.type` when the logs have one.
    pub kind: String,
    /// The newest message (first 300 characters). Selected as `sample`: an alias named `message`
    /// would shadow the column in GROUP BY.
    #[serde(rename = "sample")]
    pub message: String,
    pub bucket: u32,
    pub n: u64,
    /// Unix ms of the first and last log.
    pub first: i64,
    pub last: i64,
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
        .find(|w| w.len() > 5 && w.starts_with(|c: char| c.is_ascii_alphabetic()) && (w.ends_with("Error") || w.ends_with("Exception")) && w.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '$'))
        .unwrap_or("Error")
        .to_string()
}

/// A stable issue id (FNV-1a 64, hex): the same group gets the same id across restarts and versions.
pub(crate) fn issue_id(service: &str, error_type: &str, pattern: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [service, error_type, pattern].join("\u{1f}").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Debug)]
pub(crate) struct IssueGroup {
    pub id: String,
    pub service: String,
    pub error_type: String,
    /// The newest message of the group.
    pub message: String,
    pub count: u64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub sparkline: Vec<u64>,
}

/// Pre-aggregated error logs merged into issues (by exact message pattern), most frequent first.
pub(crate) fn group_issues(rows: Vec<ErrorRow>, buckets: usize) -> Vec<IssueGroup> {
    let mut groups: HashMap<String, IssueGroup> = HashMap::new();
    for r in rows {
        let first_line = r.message.lines().next().unwrap_or("").to_string();
        let error_type = error_type(&first_line, &r.kind);
        let id = issue_id(&r.service, &error_type, &pattern_of(&first_line));
        let g = groups.entry(id.clone()).or_insert_with(|| IssueGroup {
            id,
            service: r.service.clone(),
            error_type,
            message: first_line.clone(),
            count: 0,
            first_seen: r.first,
            last_seen: r.last,
            sparkline: vec![0; buckets],
        });
        g.count += r.n;
        g.first_seen = g.first_seen.min(r.first);
        if r.last >= g.last_seen {
            g.last_seen = r.last;
            g.message = first_line;
        }
        g.sparkline[(r.bucket as usize).min(buckets - 1)] += r.n;
    }
    let mut out: Vec<IssueGroup> = groups.into_values().collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
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

/// A saved review state (Postgres `error_issue_states`).
pub(crate) struct Saved {
    pub status: String,
    /// Unix ms of the decision.
    pub updated_at: i64,
}

const STATES: [&str; 4] = ["for_review", "reviewed", "ignored", "resolved"];

/// Issues with their status (`for_review` unless saved) and whether they regressed (resolved, then
/// seen again after the resolution), filtered to `state` (or `all`), and the totals per state.
pub(crate) fn apply_states(issues: Vec<IssueGroup>, saved: &HashMap<String, Saved>, state: &str) -> (Vec<(IssueGroup, String, bool)>, HashMap<String, usize>) {
    let mut totals: HashMap<String, usize> = STATES.iter().map(|s| (s.to_string(), 0)).collect();
    totals.insert("all".into(), issues.len());
    let mut shown = Vec::new();
    for g in issues {
        let (status, regression) = match saved.get(&g.id) {
            Some(s) if s.status == "resolved" && g.last_seen > s.updated_at => ("for_review".to_string(), true),
            Some(s) => (s.status.clone(), false),
            None => ("for_review".to_string(), false),
        };
        *totals.entry(status.clone()).or_default() += 1;
        if state == "all" || state == status {
            shown.push((g, status, regression));
        }
    }
    (shown, totals)
}

/// Pre-aggregated groups read per search (service × kind × message shape × bucket).
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
    pub assignee: Option<String>,
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

/// Issues from the organization's error logs in the range (first and last seen within it).
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/search", request_body = IssueSearchRequest,
    responses((status = 200, body = IssueSearchPage)), tag = "error-tracking")]
pub async fn search_issues(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<IssueSearchRequest>) -> ApiResult<IssueSearchPage> {
    req.range.validate()?;
    let mut issues = Vec::new();
    if matches!(req.source.as_str(), "logs" | "all") {
        let terms = parse_query(&req.query);
        let s = scope(&org, req.range, &terms, None);
        let width = ((req.range.to - req.range.from) / SPARK_BUCKETS as i64).max(1);
        // Bucket and grouping parameters first (they come first in the SQL), then the scope.
        let mut params = vec![Param::Int(req.range.from), Param::Int(width)];
        params.extend(s.params);
        params.push(Param::Int(GROUP_LIMIT));
        let rows = bind(
            st.ch(&org).query(&format!(
                "SELECT toString(service) AS service, \
                 multiIf(attr['error.kind'] != '', attr['error.kind'], attr['error.type'] != '', attr['error.type'], attr['exception.type']) AS kind, \
                 argMax(substring(message, 1, 300), timestamp) AS sample, \
                 toUInt32(intDiv(toUnixTimestamp64Milli(timestamp) - ?, ?)) AS bucket, count() AS n, \
                 toUnixTimestamp64Milli(min(timestamp)) AS first, toUnixTimestamp64Milli(max(timestamp)) AS last \
                 FROM {} WHERE {} AND status = 'error' \
                 GROUP BY service, kind, replaceRegexpAll(substring(message, 1, 200), '[0-9]+', '0'), bucket LIMIT ?",
                s.table, s.clause
            )),
            &params,
        )
        .fetch_all::<ErrorRow>()
        .await?;
        issues = group_issues(rows, SPARK_BUCKETS);
    }
    match req.sort.as_deref() {
        Some("last_seen") => issues.sort_by(|a, b| b.last_seen.cmp(&a.last_seen)),
        Some("first_seen") => issues.sort_by(|a, b| b.first_seen.cmp(&a.first_seen)),
        _ => {}
    }
    let saved: HashMap<String, Saved> = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT issue_id, status, (extract(epoch FROM updated_at) * 1000)::bigint FROM error_issue_states WHERE org_id = $1",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?
    .into_iter()
    .map(|(id, status, updated_at)| (id, Saved { status, updated_at }))
    .collect();
    let (issues, totals) = apply_states(issues, &saved, &req.state);
    let offset = req.cursor.as_deref().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
    let limit = req.limit.clamp(1, 200);
    let next_cursor = (offset + limit < issues.len()).then(|| (offset + limit).to_string());
    let page = issues
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(g, status, regression)| IssueOut {
            platform: platform_of(&g.error_type, &g.message).into(),
            persona: "backend".into(),
            source: "logs".into(),
            status,
            assignee: None,
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
        })
        .collect();
    Ok(Json(IssueSearchPage { issues: page, next_cursor, totals }))
}

#[derive(Deserialize, ToSchema)]
pub struct IssueStateUpdate {
    /// for_review, reviewed, ignored or resolved.
    pub status: String,
}

/// Marks an issue reviewed, ignored, resolved or back for review.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/{id}/state", params(("id" = String, Path)), request_body = IssueStateUpdate, responses((status = 204)), tag = "error-tracking")]
pub async fn set_issue_state(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(u): Json<IssueStateUpdate>) -> Result<StatusCode, ApiError> {
    if !STATES.contains(&u.status.as_str()) {
        return Err(ApiError::BadRequest(format!("unknown status {}", u.status)));
    }
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("invalid issue id".into()));
    }
    sqlx::query(
        "INSERT INTO error_issue_states (org_id, issue_id, status, updated_by, updated_at) VALUES ($1, $2, $3, $4::uuid, now()) \
         ON CONFLICT (org_id, issue_id) DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&u.status)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(bucket: u32, n: u64, t: i64, service: &str, message: &str) -> ErrorRow {
        ErrorRow { service: service.into(), kind: String::new(), message: message.into(), bucket, n, first: t, last: t }
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
        IssueGroup { id: id.into(), service: "billing".into(), error_type: "Error".into(), message: String::new(), count: 1, first_seen: 0, last_seen, sparkline: vec![] }
    }

    #[test]
    fn saved_states_apply_and_a_resolved_issue_that_comes_back_regresses() {
        let saved = HashMap::from([
            ("a".to_string(), Saved { status: "reviewed".into(), updated_at: 500 }),
            ("b".to_string(), Saved { status: "resolved".into(), updated_at: 500 }),
            ("c".to_string(), Saved { status: "resolved".into(), updated_at: 500 }),
        ]);
        // a: reviewed; b: resolved, not seen since; c: resolved, seen again after → for review, regression; d: unknown → for review.
        let issues = vec![group("a", 900), group("b", 400), group("c", 900), group("d", 900)];
        let (shown, totals) = apply_states(issues, &saved, "all");
        let state = |id: &str| shown.iter().find(|(g, ..)| g.id == id).map(|(_, s, r)| (s.as_str(), *r)).unwrap();
        assert_eq!(state("a"), ("reviewed", false));
        assert_eq!(state("b"), ("resolved", false));
        assert_eq!(state("c"), ("for_review", true));
        assert_eq!(state("d"), ("for_review", false));
        assert_eq!((totals["for_review"], totals["reviewed"], totals["resolved"], totals["ignored"], totals["all"]), (2, 1, 1, 0, 4));
        let (resolved, _) = apply_states(vec![group("b", 400), group("d", 900)], &saved, "resolved");
        assert_eq!(resolved.iter().map(|(g, ..)| g.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
    }

    #[test]
    fn errors_with_the_same_service_type_and_pattern_are_one_issue() {
        // Pre-aggregated per (service, kind, digit-normalized message, bucket) by ClickHouse.
        let rows = vec![
            row(0, 3, 1_000, "billing", "ValueError: order 41 not found"),
            row(1, 2, 2_000, "billing", "ValueError: order 982 not found"),
            row(2, 1, 3_000, "billing", "TimeoutError: upstream timed out after 30s"),
            row(3, 1, 4_000, "checkout", "ValueError: order 7 not found"),
        ];
        let issues = group_issues(rows, 12);
        assert_eq!(issues.len(), 3);
        let top = &issues[0];
        assert_eq!((top.service.as_str(), top.error_type.as_str(), top.count), ("billing", "ValueError", 5));
        assert_eq!((top.first_seen, top.last_seen), (1_000, 2_000));
        assert_eq!(&top.sparkline[..2], &[3, 2]);
        assert_eq!(top.sparkline.len(), 12);
        // Stable ids: the same group always gets the same id.
        assert_eq!(top.id, issue_id("billing", "ValueError", &crate::routes::logs::pattern_of("ValueError: order 41 not found")));
    }
}
