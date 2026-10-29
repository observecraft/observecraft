//! Error Tracking: error logs and APM error spans grouped into issues by service, error type and
//! message pattern.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{build_where, parse_query, Param, SpansSchema, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::Writer;
use crate::log_access::LogAccess;
use crate::error::{ApiError, ApiResult};
use crate::routes::apm::ENTRY;
use crate::routes::log_aggregates::{self, Plan, ERROR_GROUPS};
use crate::routes::logs::{bind, scope, Facet, FacetValue, TimeRange};
use crate::state::{AppState, OrgId};

/// Error logs (or spans) of one issue key (service, kind, [`shape_key`]), aggregated by ClickHouse.
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
    /// The runtime's language of the newest span (`telemetry.sdk.language`); empty for logs.
    pub lang: String,
}

/// Where an issue's errors come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Source {
    Logs,
    Apm,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Logs => "logs",
            Source::Apm => "apm",
        }
    }
}

/// The sources a search's `source` (logs | rum | apm | all) reads. RUM has none yet.
pub(crate) fn sources_of(source: &str) -> Vec<Source> {
    match source {
        "all" => vec![Source::Logs, Source::Apm],
        "logs" => vec![Source::Logs],
        "apm" => vec![Source::Apm],
        _ => Vec::new(),
    }
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

/// FNV-1a 64 of the parts, in hex.
fn fnv_hex(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.join("\u{1f}").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A stable issue id (FNV-1a 64, hex): the same group gets the same id across restarts and versions.
pub(crate) fn issue_id(service: &str, kind: &str, key: &str) -> String {
    fnv_hex(&[service, kind, key])
}

/// An APM issue's id: the source is part of the hashed key, so the same error in logs and in spans
/// makes two issues, each with its own saved state.
pub(crate) fn span_issue_id(service: &str, kind: &str, key: &str) -> String {
    fnv_hex(&["apm", service, kind, key])
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
    pub source: Source,
    /// The runtime's language when the source tells it (spans).
    pub language: String,
}

/// ClickHouse's groups (of `source`) as issues, most frequent first.
pub(crate) fn group_issues(rows: Vec<ErrorRow>, buckets: usize, source: Source) -> Vec<IssueGroup> {
    let mut out: Vec<IssueGroup> = rows
        .into_iter()
        .map(|r| {
            let first_line = strip_ansi(r.message.lines().next().unwrap_or(""));
            let mut sparkline = vec![0; buckets];
            for (b, n) in r.spark.0.iter().zip(&r.spark.1) {
                sparkline[(*b as usize).min(buckets - 1)] += n;
            }
            IssueGroup {
                id: match source {
                    Source::Logs => issue_id(&r.service, &r.kind, &r.key),
                    Source::Apm => span_issue_id(&r.service, &r.kind, &r.key),
                },
                error_type: error_type(&first_line, &r.kind),
                service: r.service,
                kind: r.kind,
                key: r.key,
                message: first_line,
                count: r.n,
                first_seen: r.first,
                last_seen: r.last,
                sparkline,
                source,
                language: r.lang,
            }
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
    out
}

/// Issues (of any sources) in the search's order: most frequent first (ties by id), or the most
/// recently last or first seen.
pub(crate) fn order_issues(mut issues: Vec<IssueGroup>, sort: Option<&str>) -> Vec<IssueGroup> {
    issues.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
    match sort {
        Some("last_seen") => issues.sort_by(|a, b| b.last_seen.cmp(&a.last_seen)),
        Some("first_seen") => issues.sort_by(|a, b| b.first_seen.cmp(&a.first_seen)),
        _ => {}
    }
    issues
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

/// The runtime of a span's `telemetry.sdk.language` (or Datadog `language` tag), when the UI knows it.
fn language_platform(language: &str) -> Option<&'static str> {
    match language.to_ascii_lowercase().as_str() {
        "java" | "jvm" => Some("java"),
        "dotnet" | ".net" | "csharp" => Some("dotnet"),
        "python" => Some("python"),
        "go" | "golang" => Some("go"),
        "nodejs" | "node" | "javascript" => Some("node"),
        _ => None,
    }
}

/// An issue's runtime: its spans' language, else what its error shows ([`platform_of`]).
pub(crate) fn issue_platform(language: &str, error_type: &str, message: &str) -> &'static str {
    language_platform(language).unwrap_or_else(|| platform_of(error_type, message))
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

/// The SQL of [`shape`] of the text `expr`.
fn shape_sql(expr: &str) -> String {
    format!("replaceRegexpAll(substringUTF8({expr}, 1, 200), '[0-9A-Fa-f]*[0-9][0-9A-Fa-f]*', '0')")
}

/// The issue key's message part: the first 80 characters of its shape. Long enough to tell errors
/// apart, short enough that variable tails (paths, quoted values) don't make one issue per log.
pub(crate) fn shape_key(message: &str) -> String {
    shape(message).chars().take(80).collect()
}

/// The SQL of [`shape_key`] of the text `expr`.
fn key_sql_of(expr: &str) -> String {
    format!("substringUTF8({}, 1, 80)", shape_sql(expr))
}

/// The SQL of [`shape_key`] (oc.logs' `error_key` column).
#[cfg(test)]
fn key_sql() -> String {
    key_sql_of("message")
}

/// The SQL of the error kind attribute (oc.logs' `error_kind` column).
#[cfg(test)]
const KIND_SQL: &str = "multiIf(attr['error.kind'] != '', attr['error.kind'], attr['error.type'] != '', attr['error.type'], attr['exception.type'])";

// ---------------------------------------------------------------- APM error spans
//
// Issues from error spans are grouped in ClickHouse like logs, by (service, error type, shape key).
// Spans have no materialized key columns: WHERE keeps only the error entry spans in range before the
// key's regex runs, and they are a small share of the spans, so computing the key per query stays
// cheap and avoids a migration of otel_traces.

/// The span's `exception` event attributes (an empty map when it has none). OpenTelemetry SDKs
/// record exceptions there; the Datadog intake maps `error.type` / `error.message` / `error.stack` to it.
const EXCEPTION: &str = "arrayElement(Events.Attributes, indexOf(Events.Name, 'exception'))";

/// The SQL of an error span's type: its exception's type, else its `error.type` attribute.
fn span_kind_sql() -> String {
    format!("if({EXCEPTION}['exception.type'] != '', {EXCEPTION}['exception.type'], SpanAttributes['error.type'])")
}

/// The SQL of an error span's message: its exception's message, else its status message, else its
/// name (the endpoint that failed).
fn span_message_sql() -> String {
    format!("multiIf({EXCEPTION}['exception.message'] != '', {EXCEPTION}['exception.message'], StatusMessage != '', StatusMessage, SpanName)")
}

/// The SQL of an error span's [`shape_key`].
fn span_key_sql() -> String {
    key_sql_of(&span_message_sql())
}

/// The SQL of the span's runtime language (OpenTelemetry resource, else Datadog's `language` tag).
const SPAN_LANGUAGE_SQL: &str = "if(ResourceAttributes['telemetry.sdk.language'] != '', ResourceAttributes['telemetry.sdk.language'], SpanAttributes['language'])";

/// The condition (after `OrgId = {oc_org:String}`) on the error spans in range that make issues:
/// service entry spans with an error status, as for APM's error groups (an error that crosses
/// services counts once per service, not once more on each caller's client span).
fn span_scope(range: TimeRange, terms: &[Term]) -> (String, Vec<Param>) {
    let f = build_where(&SpansSchema, terms, None);
    let mut params = vec![Param::Int(range.from), Param::Int(range.to)];
    params.extend(f.params);
    (format!("Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND StatusCode = 'Error' AND {ENTRY} AND ({})", f.clause), params)
}

/// One error span of an issue.
#[derive(clickhouse::Row, Deserialize)]
pub(crate) struct SpanRow {
    pub trace_id: String,
    pub span_id: String,
    pub t: i64,
    pub service: String,
    /// The span's name (its resource / endpoint).
    pub resource: String,
    pub message: String,
    /// The exception's stack trace, if recorded.
    pub stack: String,
    /// `ResourceAttributes` and `SpanAttributes` as JSON objects.
    pub resource_json: String,
    pub attributes_json: String,
}

/// Span tags by their resource attribute (the names Datadog's container tags map to).
const SPAN_TAG_ATTRS: [(&str, &str); 6] = [
    ("host", "host.name"),
    ("env", "deployment.environment"),
    ("version", "service.version"),
    ("kube_namespace", "k8s.namespace.name"),
    ("pod_name", "k8s.pod.name"),
    ("container_name", "container.name"),
];
const SPAN_TAG_KEYS: [&str; 8] = ["service", "resource_name", "env", "version", "host", "kube_namespace", "pod_name", "container_name"];

fn json_map(json: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(json).unwrap_or_default()
}

/// A span's tags: its service, resource and where it ran.
pub(crate) fn span_tags(s: &SpanRow) -> HashMap<String, String> {
    let resource = json_map(&s.resource_json);
    let mut tags: HashMap<String, String> =
        SPAN_TAG_ATTRS.iter().filter_map(|(tag, attr)| resource.get(*attr).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(|v| (tag.to_string(), v.to_string()))).collect();
    tags.insert("service".into(), s.service.clone());
    tags.insert("resource_name".into(), s.resource.clone());
    tags
}

/// A span as an issue's sample: its stack trace (else its message), tags, trace and attributes.
pub(crate) fn span_sample(s: SpanRow, error_type: &str) -> SampleOut {
    let tags = span_tags(&s);
    let message = strip_ansi(&s.message);
    let raw = if s.stack.is_empty() { message.clone() } else { strip_ansi(&s.stack) };
    let mut resource = json_map(&s.resource_json);
    resource.retain(|k, _| !k.starts_with("oc."));
    let json = serde_json::json!({
        "service": s.service,
        "resource": resource,
        "resource_name": s.resource,
        "trace_id": s.trace_id,
        "span_id": s.span_id,
        "error": { "type": error_type, "message": message, "stack": s.stack },
        "attributes": json_map(&s.attributes_json),
    });
    let trace = serde_json::json!({ "name": "Trace", "attributes": [
        { "label": "Trace Id", "value": s.trace_id },
        { "label": "Span Id", "value": s.span_id },
        { "label": "Resource", "value": s.resource },
    ] });
    SampleOut {
        id: s.span_id.clone(),
        timestamp: s.t,
        tags,
        stack: StackOut { title: format!("{error_type}: {}", message.lines().next().unwrap_or("")), frames: Vec::new(), raw },
        timeline: Vec::new(),
        attribute_groups: vec![trace],
        json,
        trace_id: Some(s.trace_id),
        span_id: Some(s.span_id),
    }
}

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

/// The SQL of the error groups of `plan` (error logs of the aggregate's window and the rest of the
/// range from the logs), with `buckets` sparkline buckets of `width` ms from `from`; `only` narrows
/// to one issue's kind and key.
pub(crate) fn issues_query(plan: &Plan, from: i64, width: i64, only: Option<(&str, &str)>, limit: i64) -> (String, Vec<Param>) {
    let narrowing = if only.is_some() { " AND error_kind = ? AND error_key = ?" } else { "" };
    let only_params = || only.map(|(kind, key)| vec![Param::Str(kind.into()), Param::Str(key.into())]).unwrap_or_default();
    let mut parts = Vec::new();
    let mut params = Vec::new();
    if let Some(agg) = &plan.agg {
        parts.push(format!(
            "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, intDiv(toInt64(toUnixTimestamp(minute)) * 1000 - ?, ?) AS b, \
             sum(n) AS c, min(first_seen) AS f, max(last_seen) AS l, argMaxMergeState(sample) AS state \
             FROM {} WHERE {}{narrowing} GROUP BY service, kind, key, b",
            agg.table, agg.clause
        ));
        params.extend([Param::Int(from), Param::Int(width)]);
        params.extend(agg.params.iter().cloned());
        params.extend(only_params());
    }
    parts.push(format!(
        "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, intDiv(toUnixTimestamp64Milli(timestamp) - ?, ?) AS b, \
         count() AS c, min(timestamp) AS f, max(timestamp) AS l, argMaxState(substringUTF8(message, 1, 300), timestamp) AS state \
         FROM {} WHERE {} AND status = 'error'{narrowing} GROUP BY service, kind, key, b",
        plan.logs.table, plan.logs.clause
    ));
    params.extend([Param::Int(from), Param::Int(width)]);
    params.extend(plan.logs.params.iter().cloned());
    params.extend(only_params());
    params.push(Param::Int(limit));
    let sql = format!(
        "SELECT service, kind, key, argMaxMerge(state) AS sample, sum(c) AS n, toUnixTimestamp64Milli(min(f)) AS first, toUnixTimestamp64Milli(max(l)) AS last, \
         sumMap([toUInt32(greatest(b, 0))], [c]) AS spark, '' AS lang FROM ({}) GROUP BY service, kind, key ORDER BY n DESC LIMIT ?",
        parts.join(" UNION ALL ")
    );
    (sql, params)
}

/// The organization's issues from error logs in `range` (matching `query`), `buckets` per sparkline;
/// with `only`, just that issue's logs.
async fn load_issues(st: &AppState, org: &str, access: &LogAccess, range: TimeRange, query: &str, buckets: usize, only: Option<&IssueGroup>) -> Result<Vec<IssueGroup>, ApiError> {
    let mut terms = parse_query(query);
    if let Some(g) = only {
        terms.push(oc_query::Term::Tag { key: "service".into(), values: vec![g.service.clone()], negate: false });
    }
    let since = log_aggregates::since(&st.ch(org), org, &ERROR_GROUPS).await?;
    let plan = log_aggregates::plan(org, access, &ERROR_GROUPS, range, &terms, None, since, &[]);
    let width = ((range.to - range.from) / buckets as i64).max(1);
    let (sql, params) = issues_query(&plan, range.from, width, only.map(|g| (g.kind.as_str(), g.key.as_str())), GROUP_LIMIT);
    let rows = bind(st.ch(org).query(&sql), &params).fetch_all::<ErrorRow>().await?;
    Ok(group_issues(rows, buckets, Source::Logs))
}

/// The organization's issues from APM error spans in `range` (matching `query`), `buckets` per
/// sparkline; with `only`, just that issue's spans.
async fn load_span_issues(st: &AppState, org: &str, range: TimeRange, query: &str, buckets: usize, only: Option<&IssueGroup>) -> Result<Vec<IssueGroup>, ApiError> {
    let (clause, scope_params) = span_scope(range, &parse_query(query));
    let width = ((range.to - range.from) / buckets as i64).max(1);
    // Bucket parameters first (they come first in the SQL), then the scope.
    let mut params = vec![Param::Int(range.from), Param::Int(width)];
    params.extend(scope_params);
    let (kind, key, message) = (span_kind_sql(), span_key_sql(), span_message_sql());
    let narrowing = match only {
        Some(g) => {
            params.extend([Param::Str(g.service.clone()), Param::Str(g.kind.clone()), Param::Str(g.key.clone())]);
            format!(" AND ServiceName = ? AND {kind} = ? AND {key} = ?")
        }
        None => String::new(),
    };
    params.push(Param::Int(GROUP_LIMIT));
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(ServiceName) AS service, {kind} AS kind, {key} AS key, \
             argMax(substringUTF8({message}, 1, 300), Timestamp) AS sample, count() AS n, \
             toUnixTimestamp64Milli(min(Timestamp)) AS first, toUnixTimestamp64Milli(max(Timestamp)) AS last, \
             sumMap([toUInt32(intDiv(toUnixTimestamp64Milli(Timestamp) - ?, ?))], [toUInt64(1)]) AS spark, argMax({SPAN_LANGUAGE_SQL}, Timestamp) AS lang \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause}{narrowing} \
             GROUP BY service, kind, key ORDER BY n DESC LIMIT ?"
        )),
        &params,
    )
    .fetch_all::<ErrorRow>()
    .await?;
    Ok(group_issues(rows, buckets, Source::Apm))
}

/// The issues of one source.
#[allow(clippy::too_many_arguments)]
async fn load_source(st: &AppState, org: &str, access: &LogAccess, source: Source, range: TimeRange, query: &str, buckets: usize, only: Option<&IssueGroup>) -> Result<Vec<IssueGroup>, ApiError> {
    match source {
        Source::Logs => load_issues(st, org, access, range, query, buckets, only).await,
        Source::Apm => load_span_issues(st, org, range, query, buckets, only).await,
    }
}

/// The issue `id` in range, from whichever source has it.
async fn find_issue(st: &AppState, org: &str, access: &LogAccess, range: TimeRange, query: &str, id: &str) -> Result<Option<IssueGroup>, ApiError> {
    for source in [Source::Logs, Source::Apm] {
        if let Some(g) = load_source(st, org, access, source, range, query, 1, None).await?.into_iter().find(|g| g.id == id) {
            return Ok(Some(g));
        }
    }
    Ok(None)
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
        platform: issue_platform(&g.language, &g.error_type, &g.message).into(),
        persona: "backend".into(),
        source: g.source.name().into(),
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

/// Open issues (for review or reviewed) per service in the range, as `access` may read them
/// (Developer Portal home).
pub(crate) async fn open_issues_by_service(st: &AppState, org: &str, access: &LogAccess, range: TimeRange) -> Result<HashMap<String, u64>, ApiError> {
    let mut issues = Vec::new();
    for source in sources_of("all") {
        issues.extend(load_source(st, org, access, source, range, "", 1, None).await?);
    }
    let saved = saved_states(st, org).await?;
    let (stated, _) = apply_states(issues, &saved, "all");
    let mut out: HashMap<String, u64> = HashMap::new();
    for i in stated.into_iter().filter(|i| i.status == "for_review" || i.status == "reviewed") {
        *out.entry(i.group.service).or_default() += 1;
    }
    Ok(out)
}

/// Issues from the organization's error logs and APM error spans in the range (first and last seen within it).
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/search", request_body = IssueSearchRequest,
    responses((status = 200, body = IssueSearchPage)), tag = "error-tracking")]
pub async fn search_issues(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<IssueSearchRequest>) -> ApiResult<IssueSearchPage> {
    req.range.validate()?;
    let mut issues = Vec::new();
    for source in sources_of(&req.source) {
        issues.extend(load_source(&st, &org, &access, source, req.range, &req.query, SPARK_BUCKETS, None).await?);
    }
    let issues = order_issues(issues, req.sort.as_deref());
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
async fn issue_logs(st: &AppState, org: &str, access: &LogAccess, range: TimeRange, query: &str, group: &IssueGroup, oldest: bool) -> Result<Vec<LogRow>, ApiError> {
    let mut terms = parse_query(query);
    terms.push(oc_query::Term::Tag { key: "service".into(), values: vec![group.service.clone()], negate: false });
    let s = scope(org, access, range, &terms, None);
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

/// The newest (or oldest) error spans in range that belong to the issue (its service, type and message shape).
async fn issue_spans(st: &AppState, org: &str, range: TimeRange, query: &str, group: &IssueGroup, oldest: bool) -> Result<Vec<SpanRow>, ApiError> {
    let (clause, mut params) = span_scope(range, &parse_query(query));
    params.extend([Param::Str(group.service.clone()), Param::Str(group.kind.clone()), Param::Str(group.key.clone()), Param::Int(ISSUE_LOGS)]);
    let (kind, key, message) = (span_kind_sql(), span_key_sql(), span_message_sql());
    Ok(bind(
        st.ch(org).query(&format!(
            "SELECT TraceId AS trace_id, SpanId AS span_id, toUnixTimestamp64Milli(Timestamp) AS t, toString(ServiceName) AS service, \
             toString(SpanName) AS resource, substringUTF8({message}, 1, 5000) AS message, \
             substringUTF8({EXCEPTION}['exception.stacktrace'], 1, 20000) AS stack, \
             toJSONString(ResourceAttributes) AS resource_json, toJSONString(SpanAttributes) AS attributes_json \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND ServiceName = ? AND {kind} = ? AND {key} = ? \
             ORDER BY Timestamp {} LIMIT ?",
            if oldest { "ASC" } else { "DESC" }
        )),
        &params,
    )
    .fetch_all::<SpanRow>()
    .await?)
}

fn valid_issue_id(id: &str) -> Result<(), ApiError> {
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("invalid issue id".into()));
    }
    Ok(())
}

/// One issue in the range: its histogram, the tags of its logs (or spans) and its saved state.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/{id}", params(("id" = String, Path)), request_body = IssueDetailRequest, responses((status = 200, body = IssueDetailOut)), tag = "error-tracking")]
pub async fn issue_detail(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Path(id): Path<String>, Json(req): Json<IssueDetailRequest>) -> ApiResult<IssueDetailOut> {
    valid_issue_id(&id)?;
    req.range.validate()?;
    let found = find_issue(&st, &org, &access, req.range, &req.query, &id).await?.ok_or_else(|| ApiError::NotFound("no such issue in this time range".into()))?;
    // Its own errors only, for exact counts and a finer histogram.
    let group = load_source(&st, &org, &access, found.source, req.range, &req.query, DETAIL_BUCKETS, Some(&found)).await?.into_iter().find(|g| g.id == id).unwrap_or(found);
    let width = ((req.range.to - req.range.from) / DETAIL_BUCKETS as i64).max(1);
    let histogram = group.sparkline.iter().enumerate().map(|(i, n)| HistogramPoint { t: req.range.from + i as i64 * width, count: *n }).collect();
    let tags = match group.source {
        Source::Logs => {
            let logs = issue_logs(&st, &org, &access, req.range, &req.query, &group, false).await?;
            let tag_maps: Vec<HashMap<String, String>> = logs.iter().map(|l| serde_json::from_str(&l.tags_json).unwrap_or_default()).collect();
            tag_distribution(&tag_maps, &TAG_KEYS)
        }
        Source::Apm => {
            let spans = issue_spans(&st, &org, req.range, &req.query, &group, false).await?;
            tag_distribution(&spans.iter().map(span_tags).collect::<Vec<_>>(), &SPAN_TAG_KEYS)
        }
    };
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
    /// The trace and span of an APM sample, for a link to the trace.
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct SamplePage {
    pub sample: Option<SampleOut>,
    pub index: usize,
    /// The issue's logs (or spans) in range.
    pub total: u64,
    /// Logs (or spans) that can be paged through (the newest or oldest 2,000).
    pub available: usize,
}

/// One log (or span) of the issue (the `index`-th newest or oldest in range), and how many there are.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/{id}/samples", params(("id" = String, Path)), request_body = SampleRequest, responses((status = 200, body = SamplePage)), tag = "error-tracking")]
pub async fn issue_samples(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Path(id): Path<String>, Json(req): Json<SampleRequest>) -> ApiResult<SamplePage> {
    valid_issue_id(&id)?;
    req.range.validate()?;
    let Some(group) = find_issue(&st, &org, &access, req.range, &req.query, &id).await? else {
        return Ok(Json(SamplePage { sample: None, index: 0, total: 0, available: 0 }));
    };
    let oldest = req.order.as_deref() == Some("oldest");
    if group.source == Source::Apm {
        let spans = issue_spans(&st, &org, req.range, &req.query, &group, oldest).await?;
        let available = spans.len();
        let index = req.index.min(available.saturating_sub(1));
        let sample = spans.into_iter().nth(index).map(|s| span_sample(s, &group.error_type));
        return Ok(Json(SamplePage { sample, index, total: group.count, available }));
    }
    let logs = issue_logs(&st, &org, &access, req.range, &req.query, &group, oldest).await?;
    let available = logs.len();
    let index = req.index.min(available.saturating_sub(1));
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
            trace_id: None,
            span_id: None,
        }
    });
    Ok(Json(SamplePage { sample, index, total: group.count, available }))
}

/// Facets of the issue list: the log fields an issue search can filter on.
const ISSUE_FACETS: [(&str, &str); 3] = [("service", "Service"), ("env", "Env"), ("version", "Version")];

#[derive(clickhouse::Row, Deserialize)]
pub(crate) struct FacetRow {
    pub k: String,
    pub v: String,
    /// Issues with logs of this value.
    pub n: u64,
}

/// Facets in [`ISSUE_FACETS`] order, most issues first, without empty values. Rows of several
/// sources add up (their issues are distinct).
pub(crate) fn issue_facets_of(rows: Vec<FacetRow>) -> Vec<Facet> {
    let mut facets: Vec<Facet> = ISSUE_FACETS.iter().map(|(key, label)| Facet { key: key.to_string(), label: label.to_string(), group: String::new(), values: Vec::new(), approximate: false }).collect();
    for r in rows.into_iter().filter(|r| !r.v.is_empty()) {
        if let Some(f) = facets.iter_mut().find(|f| f.key == r.k) {
            match f.values.iter_mut().find(|v| v.value == r.v) {
                Some(v) => v.count += r.n,
                None => f.values.push(FacetValue { value: r.v, count: r.n }),
            }
        }
    }
    for f in &mut facets {
        f.values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
        f.values.truncate(FACET_VALUES as usize);
    }
    facets
}

#[derive(Deserialize, ToSchema)]
pub struct IssueFacetsRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// logs | rum | apm | all
    #[serde(default = "all_state")]
    pub source: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueFacetsOut {
    pub facets: Vec<Facet>,
    pub total_available: usize,
}

/// Values per facet in range: 20 per facet.
const FACET_VALUES: i64 = 20;

/// The SQL of the log issues per service, env and version of `plan`, `limit` values per facet.
pub(crate) fn issue_facets_query(plan: &Plan, limit: i64) -> (String, Vec<Param>) {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    if let Some(agg) = &plan.agg {
        parts.push(format!(
            "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, toString(env) AS env, toString(version) AS version \
             FROM {} WHERE {} GROUP BY service, kind, key, env, version",
            agg.table, agg.clause
        ));
        params.extend(agg.params.iter().cloned());
    }
    parts.push(format!(
        "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, tags['env'] AS env, tags['version'] AS version \
         FROM {} WHERE {} AND status = 'error' GROUP BY service, kind, key, env, version",
        plan.logs.table, plan.logs.clause
    ));
    params.extend(plan.logs.params.iter().cloned());
    params.push(Param::Int(limit));
    let sql = format!(
        "SELECT kv.1 AS k, kv.2 AS v, uniqExact(service, kind, key) AS n FROM ({}) \
         ARRAY JOIN [('service', service), ('env', env), ('version', version)] AS kv GROUP BY k, v ORDER BY n DESC LIMIT ? BY k",
        parts.join(" UNION ALL ")
    );
    (sql, params)
}

/// Log issues per service, env and version.
async fn log_facet_rows(st: &AppState, org: &str, access: &LogAccess, range: TimeRange, query: &str) -> Result<Vec<FacetRow>, ApiError> {
    let since = log_aggregates::since(&st.ch(org), org, &ERROR_GROUPS).await?;
    let plan = log_aggregates::plan(org, access, &ERROR_GROUPS, range, &parse_query(query), None, since, &["service", "env", "version"]);
    let (sql, params) = issue_facets_query(&plan, FACET_VALUES);
    Ok(bind(st.ch(org).query(&sql), &params).fetch_all::<FacetRow>().await?)
}

/// APM issues per service, env and version.
async fn span_facet_rows(st: &AppState, org: &str, range: TimeRange, query: &str) -> Result<Vec<FacetRow>, ApiError> {
    let (clause, mut params) = span_scope(range, &parse_query(query));
    params.push(Param::Int(FACET_VALUES));
    let (kind, key) = (span_kind_sql(), span_key_sql());
    Ok(bind(
        st.ch(org).query(&format!(
            "SELECT kv.1 AS k, kv.2 AS v, uniqExact(ServiceName, {kind}, {key}) AS n \
             FROM otel.otel_traces ARRAY JOIN [('service', toString(ServiceName)), ('env', ResourceAttributes['deployment.environment']), \
             ('version', ResourceAttributes['service.version'])] AS kv \
             WHERE OrgId = {{oc_org:String}} AND {clause} GROUP BY k, v ORDER BY n DESC LIMIT ? BY k"
        )),
        &params,
    )
    .fetch_all::<FacetRow>()
    .await?)
}

/// How many issues have logs (or spans) with each service, env and version.
#[utoipa::path(post, path = "/api/v1/error-tracking/issues/facets", request_body = IssueFacetsRequest, responses((status = 200, body = IssueFacetsOut)), tag = "error-tracking")]
pub async fn issue_facets(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<IssueFacetsRequest>) -> ApiResult<IssueFacetsOut> {
    req.range.validate()?;
    let mut rows = Vec::new();
    for source in sources_of(&req.source) {
        rows.extend(match source {
            Source::Logs => log_facet_rows(&st, &org, &access, req.range, &req.query).await?,
            Source::Apm => span_facet_rows(&st, &org, req.range, &req.query).await?,
        });
    }
    Ok(Json(IssueFacetsOut { facets: issue_facets_of(rows), total_available: ISSUE_FACETS.len() }))
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
        ErrorRow { service: service.into(), kind: String::new(), key: shape_key(message), message: message.into(), n, first, last, spark, lang: String::new() }
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
            source: Source::Logs,
            language: String::new(),
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
        let issues = group_issues(vec![row("billing", "\u{1b}[31mValueError\u{1b}[0m: bad", 1, 0, 0, (vec![0], vec![1]))], 1, Source::Logs);
        assert_eq!(issues[0].message, "ValueError: bad");
    }

    #[test]
    fn facets_count_issues_per_value_in_a_fixed_order() {
        let rows = vec![
            FacetRow { k: "env".into(), v: "prod".into(), n: 3 },
            FacetRow { k: "service".into(), v: "billing".into(), n: 2 },
            FacetRow { k: "service".into(), v: "checkout".into(), n: 5 },
            FacetRow { k: "env".into(), v: String::new(), n: 9 },
        ];
        let facets = issue_facets_of(rows);
        assert_eq!(facets.iter().map(|f| (f.key.as_str(), f.label.as_str())).collect::<Vec<_>>(), vec![("service", "Service"), ("env", "Env"), ("version", "Version")]);
        let values = |i: usize| facets[i].values.iter().map(|v| (v.value.as_str(), v.count)).collect::<Vec<_>>();
        assert_eq!(values(0), vec![("checkout", 5), ("billing", 2)]);
        // Logs without the tag don't make an empty value.
        assert_eq!(values(1), vec![("prod", 3)]);
        assert!(values(2).is_empty());
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
    fn a_search_reads_the_sources_it_names() {
        assert_eq!(sources_of("all"), vec![Source::Logs, Source::Apm]);
        assert_eq!(sources_of("logs"), vec![Source::Logs]);
        assert_eq!(sources_of("apm"), vec![Source::Apm]);
        // No RUM product yet: no issues rather than other sources' issues.
        assert!(sources_of("rum").is_empty());
    }

    #[test]
    fn span_issue_ids_are_stable_and_apart_from_log_issue_ids() {
        let key = shape_key("ValueError: order 41 not found");
        let id = span_issue_id("billing", "ValueError", &key);
        assert_eq!(id, span_issue_id("billing", "ValueError", &shape_key("ValueError: order 982 not found")));
        assert_eq!((id.len(), id.chars().all(|c| c.is_ascii_hexdigit())), (16, true));
        // The same error in logs and in spans makes two issues, with their own saved states.
        assert_ne!(id, issue_id("billing", "ValueError", &key));
        assert_ne!(id, span_issue_id("checkout", "ValueError", &key));
        // Log issue ids don't change (saved states are keyed by them).
        assert_eq!(issue_id("billing", "", "x"), "fd91b72e354d5ffe");
        let mut r = row("billing", "ValueError: order 41 not found", 2, 0, 0, (vec![0], vec![2]));
        r.kind = "ValueError".into();
        let issues = group_issues(vec![r], 1, Source::Apm);
        assert_eq!((issues[0].id.as_str(), issues[0].source), (id.as_str(), Source::Apm));
    }

    #[test]
    fn the_span_language_names_the_platform() {
        assert_eq!(issue_platform("java", "Error", "boom"), "java");
        assert_eq!(issue_platform("nodejs", "Error", "boom"), "node");
        assert_eq!(issue_platform("dotnet", "Error", "boom"), "dotnet");
        assert_eq!(issue_platform("python", "Error", "boom"), "python");
        assert_eq!(issue_platform("go", "Error", "boom"), "go");
        // Without a known language, the error itself tells (or nothing does).
        assert_eq!(issue_platform("", "java.lang.IllegalStateException", ""), "java");
        assert_eq!(issue_platform("ruby", "Error", "boom"), "unknown");
    }

    #[test]
    fn spans_compute_the_issue_key_like_logs() {
        // The span key is the logs' key SQL over the span's message (exception message, status message, span name).
        assert_eq!(key_sql_of("message"), key_sql());
        let key = span_key_sql();
        assert!(key.starts_with("substringUTF8(replaceRegexpAll(substringUTF8(multiIf("));
        assert!(span_message_sql().contains("['exception.message']") && span_message_sql().contains("StatusMessage"));
        assert!(span_kind_sql().contains("['exception.type']"));
    }

    fn span(message: &str, stack: &str) -> SpanRow {
        SpanRow {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".into(),
            span_id: "00f067aa0ba902b7".into(),
            t: 1_000,
            service: "billing".into(),
            resource: "POST /invoices".into(),
            message: message.into(),
            stack: stack.into(),
            resource_json: r#"{"service.name":"billing","deployment.environment":"prod","service.version":"1.4.2","host.name":"i-0a1b2c3d4e5f60003","k8s.pod.name":"billing-7d9f8c6b5-x2x4q","oc.org_id":"default"}"#.into(),
            attributes_json: r#"{"http.route":"/invoices"}"#.into(),
        }
    }

    #[test]
    fn span_tags_name_the_service_resource_and_where_it_ran() {
        let tags = span_tags(&span("boom", ""));
        let get = |k: &str| tags.get(k).map(String::as_str);
        assert_eq!((get("service"), get("resource_name"), get("env"), get("version")), (Some("billing"), Some("POST /invoices"), Some("prod"), Some("1.4.2")));
        assert_eq!((get("host"), get("pod_name"), get("kube_namespace")), (Some("i-0a1b2c3d4e5f60003"), Some("billing-7d9f8c6b5-x2x4q"), None));
    }

    #[test]
    fn a_span_sample_has_its_stack_and_trace() {
        let s = span_sample(span("invoice 42 locked\nretry later", "InvoiceLocked: invoice 42 locked\n    at billing.handler"), "InvoiceLocked");
        assert_eq!((s.trace_id.as_deref(), s.span_id.as_deref()), (Some("4bf92f3577b34da6a3ce929d0e0e4736"), Some("00f067aa0ba902b7")));
        assert_eq!(s.stack.title, "InvoiceLocked: invoice 42 locked");
        assert_eq!(s.stack.raw, "InvoiceLocked: invoice 42 locked\n    at billing.handler");
        assert_eq!((s.id.as_str(), s.timestamp), ("00f067aa0ba902b7", 1_000));
        assert_eq!(s.tags.get("env").map(String::as_str), Some("prod"));
        assert_eq!(s.json["trace_id"], "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(s.json["attributes"]["http.route"], "/invoices");
        assert_eq!(s.json["error"]["message"], "invoice 42 locked\nretry later");
        // Internal attributes stay internal.
        assert!(s.json["resource"].get("oc.org_id").is_none());
        // Without a stack trace, the message stands in for it.
        assert_eq!(span_sample(span("timeout", ""), "Error").stack.raw, "timeout");
    }

    #[test]
    fn issues_of_both_sources_merge_in_one_order() {
        let issue = |id: &str, source: Source, count: u64, first_seen: i64, last_seen: i64| IssueGroup { source, count, first_seen, ..group(id, last_seen) };
        let issues = || vec![issue("a", Source::Logs, 3, 10, 50), issue("b", Source::Apm, 9, 20, 40), issue("c", Source::Apm, 3, 30, 60)];
        let ids = |v: Vec<IssueGroup>| v.into_iter().map(|g| g.id).collect::<Vec<_>>();
        // Most frequent first (ties by id), as for logs alone.
        assert_eq!(ids(order_issues(issues(), None)), vec!["b", "a", "c"]);
        assert_eq!(ids(order_issues(issues(), Some("last_seen"))), vec!["c", "a", "b"]);
        assert_eq!(ids(order_issues(issues(), Some("first_seen"))), vec!["c", "b", "a"]);
    }

    #[test]
    fn facet_counts_of_both_sources_add_up() {
        let rows = vec![
            FacetRow { k: "service".into(), v: "billing".into(), n: 2 },
            FacetRow { k: "service".into(), v: "checkout".into(), n: 3 },
            FacetRow { k: "service".into(), v: "billing".into(), n: 4 },
        ];
        let facets = issue_facets_of(rows);
        assert_eq!(facets[0].values.iter().map(|v| (v.value.as_str(), v.count)).collect::<Vec<_>>(), vec![("billing", 6), ("checkout", 3)]);
    }

    /// Error groups and their facets read from the error aggregate (and the logs around it) are
    /// those read from the logs alone, as before the aggregate. Skipped unless OC_TEST_CLICKHOUSE_URL
    /// points at a ClickHouse server (user `default`, or OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).
    #[tokio::test]
    async fn error_groups_from_the_aggregate_match_the_logs() {
        let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
            eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
            return;
        };
        let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
        let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
        let cfg = oc_store::ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: None };
        crate::test_support::migrate_clickhouse_once(&cfg).await;
        let ch = clickhouse::Client::default().with_url(&url).with_user(&user).with_password(&password).with_database("oc");
        let org = format!("error-parity-{}", std::process::id());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let t0 = now / 60_000 * 60_000 - 30 * 60_000;
        // 12 minutes of logs every 97 ms: errors of 3 services, kinds from attributes and messages, 2 envs and versions.
        ch.query(&format!(
            "INSERT INTO oc.logs (org_id, timestamp, status, service, host, source, message, tags, attr, retention_days) \
             SELECT '{org}', fromUnixTimestamp64Milli(toInt64({t0} + number * 97)), if(number % 3 = 0, 'info', 'error'), \
                    ['billing', 'checkout', 'order-service'][number % 3 + 1], concat('i-0a1b2c3d4e5f6000', toString(number % 4)), 'python', \
                    [concat('ValueError: order ', toString(number), ' not found'), concat('upstream timed out after ', toString(number % 30), 's'), \
                     concat('java.lang.IllegalStateException: invoice ', toString(number % 7), ' locked')][number % 5 % 3 + 1], \
                    map('env', if(number % 4 = 0, 'staging', 'prod'), 'version', concat('1.0.', toString(number % 2)), 'pod_name', concat('pod-', toString(number % 3))), \
                    if(number % 5 = 2, map('error.kind', 'java.lang.IllegalStateException'), map()), 15 \
             FROM numbers(7400)"
        ))
        .execute()
        .await
        .expect("insert logs");

        #[derive(clickhouse::Row, Deserialize, Debug, PartialEq, PartialOrd)]
        struct Facet3 {
            k: String,
            v: String,
            n: u64,
        }
        let groups = |sql: String, params: Vec<Param>| {
            let ch = ch.clone();
            async move {
                let mut rows: Vec<_> = bind(ch.query(&sql), &params)
                    .fetch_all::<ErrorRow>()
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"))
                    .into_iter()
                    .map(|r| (r.service, r.kind, r.key, r.message, r.n, r.first, r.last, r.spark.1.iter().sum::<u64>()))
                    .collect();
                rows.sort();
                rows
            }
        };
        let facets = |sql: String, params: Vec<Param>| {
            let ch = ch.clone();
            async move {
                let mut rows = bind(ch.query(&sql), &params).fetch_all::<Facet3>().await.unwrap_or_else(|e| panic!("{sql}: {e}"));
                rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
                rows
            }
        };
        let restricted = |q: &str| {
            let rule = crate::log_access::Restriction { terms: parse_query(q), roles: ["r".to_string()].into(), teams: Default::default() };
            LogAccess::resolve(&[rule], false, &["r".to_string()], &[])
        };
        // Partial minutes at both ends.
        let range = TimeRange { from: t0 + 30_500, to: t0 + 11 * 60_000 + 20_250 };
        let width = (range.to - range.from) / 12;
        let check = |since: Option<i64>| {
            let (groups, facets, org, restricted) = (&groups, &facets, org.clone(), &restricted);
            async move {
                let mut used = 0;
                for (query, access) in [
                    ("", LogAccess::org_background()),
                    ("env:prod", LogAccess::org_background()),
                    ("service:checkout -version:1.0.1", LogAccess::org_background()),
                    ("service:checkout timed", LogAccess::org_background()),
                    ("", restricted("env:staging")),
                    ("", restricted("pod_name:pod-1")),
                ] {
                    let terms = parse_query(query);
                    let p = log_aggregates::plan(&org, &access, &ERROR_GROUPS, range, &terms, None, since, &[]);
                    used += usize::from(p.agg.is_some());
                    // Today's queries, over the logs.
                    let s = scope(&org, &access, range, &terms, None);
                    let mut reference_params = vec![Param::Int(range.from), Param::Int(width)];
                    reference_params.extend(s.params.clone());
                    reference_params.push(Param::Int(GROUP_LIMIT));
                    let reference = format!(
                        "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, \
                         argMax(substringUTF8(message, 1, 300), timestamp) AS sample, count() AS n, \
                         toUnixTimestamp64Milli(min(timestamp)) AS first, toUnixTimestamp64Milli(max(timestamp)) AS last, \
                         sumMap([toUInt32(intDiv(toUnixTimestamp64Milli(timestamp) - ?, ?))], [toUInt64(1)]) AS spark, '' AS lang \
                         FROM {} WHERE {} AND status = 'error' GROUP BY service, error_kind, error_key ORDER BY n DESC LIMIT ?",
                        s.table, s.clause
                    );
                    let expected = groups(reference, reference_params).await;
                    assert!(!expected.is_empty() , "{query}: no fixtures");
                    let (sql, params) = issues_query(&p, range.from, width, None, GROUP_LIMIT);
                    assert_eq!(groups(sql, params).await, expected, "groups of {query:?} (aggregate: {})", p.agg.is_some());
                    // One issue's own logs.
                    if let Some((_, kind, key, ..)) = expected.first() {
                        let (sql, params) = issues_query(&p, range.from, width, Some((kind, key)), GROUP_LIMIT);
                        let only: Vec<_> = expected.iter().filter(|g| &g.1 == kind && &g.2 == key).cloned().collect();
                        assert_eq!(groups(sql, params).await, only, "one issue of {query:?}");
                    }
                    let mut reference_params = s.params.clone();
                    reference_params.push(Param::Int(FACET_VALUES));
                    let reference = format!(
                        "SELECT kv.1 AS k, kv.2 AS v, uniqExact(service, error_kind, error_key) AS n \
                         FROM {} ARRAY JOIN [('service', toString(service)), ('env', tags['env']), ('version', tags['version'])] AS kv \
                         WHERE {} AND status = 'error' GROUP BY k, v ORDER BY n DESC LIMIT ? BY k",
                        s.table, s.clause
                    );
                    let (sql, params) = issue_facets_query(&p, FACET_VALUES);
                    assert_eq!(facets(sql, params).await, facets(reference, reference_params).await, "facets of {query:?}");
                }
                used
            }
        };
        let since = log_aggregates::since(&ch, &org, &ERROR_GROUPS).await.unwrap();
        assert_eq!(since, Some(t0 + 60_000), "the first minute may be partial");
        // The aggregate answers the searches and restrictions it can express (4 of 6).
        assert_eq!(check(since).await, 4);
        // Logs from before the aggregate (here: its first minutes dropped) come from the logs.
        ch.query("ALTER TABLE oc.error_groups_1m DELETE WHERE org_id = ? AND minute < fromUnixTimestamp64Milli(?) SETTINGS mutations_sync = 2").bind(&org).bind(t0 + 4 * 60_000).execute().await.unwrap();
        assert_eq!(check(Some(t0 + 5 * 60_000)).await, 4);
        // No aggregate yet: the logs alone.
        assert_eq!(check(None).await, 0);
    }

    #[test]
    fn each_clickhouse_key_is_one_issue_with_its_sparkline() {
        // One row per (service, kind, shape key), aggregated by ClickHouse with its buckets.
        let rows = vec![
            row("billing", "ValueError: order 41 not found", 5, 1_000, 2_000, (vec![0, 1], vec![3, 2])),
            row("billing", "TimeoutError: upstream timed out after 30s", 1, 3_000, 3_000, (vec![2], vec![1])),
            row("checkout", "ValueError: order 7 not found", 1, 4_000, 4_000, (vec![3], vec![1])),
        ];
        let issues = group_issues(rows, 12, Source::Logs);
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
