//! Logs endpoints backed by `oc.logs` in ClickHouse. Shapes mirror `web/src/api/types.ts`.

use std::collections::HashMap;

use axum::extract::State;
use axum::Json;
use futures::future::try_join_all;
use oc_query::{build_where, parse_query, LogsSchema, Param, Schema, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const MAX_RANGE_MS: i64 = 31 * 24 * 3600 * 1000;
const MAX_LIMIT: u64 = 1000;

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy)]
pub struct TimeRange {
    /// Epoch milliseconds (inclusive).
    pub from: i64,
    /// Epoch milliseconds (inclusive).
    pub to: i64,
}

impl TimeRange {
    pub(crate) fn validate(&self) -> Result<(), ApiError> {
        if self.to < self.from {
            return Err(ApiError::BadRequest("range.to must be >= range.from".into()));
        }
        if self.to - self.from > MAX_RANGE_MS {
            return Err(ApiError::BadRequest("range may not exceed 31 days".into()));
        }
        Ok(())
    }

    /// For data kept up to `max_days` and cheap to read over long periods (metric rollups).
    pub(crate) fn validate_within(&self, max_days: u32) -> Result<(), ApiError> {
        if self.to < self.from {
            return Err(ApiError::BadRequest("range.to must be >= range.from".into()));
        }
        if self.to - self.from > max_days as i64 * 86_400_000 {
            return Err(ApiError::BadRequest(format!("range may not exceed {max_days} days")));
        }
        Ok(())
    }

    /// For small metadata tables (incidents) that can be listed over long periods: up to 2 years.
    pub(crate) fn validate_loose(&self) -> Result<(), ApiError> {
        if self.to < self.from {
            return Err(ApiError::BadRequest("range.to must be >= range.from".into()));
        }
        if self.to - self.from > 2 * 366 * 86_400_000 {
            return Err(ApiError::BadRequest("range may not exceed 2 years".into()));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub id: String,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub status: String,
    pub service: String,
    pub host: String,
    pub source: String,
    pub message: String,
    pub tags: HashMap<String, String>,
    #[schema(value_type = Object)]
    pub attributes: serde_json::Value,
    /// Trace the log was written in (OTLP logs, or `trace_id` in JSON logs); links to APM.
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

#[derive(clickhouse::Row, Deserialize)]
struct LogRow {
    id: String,
    ts: i64,
    status: String,
    service: String,
    host: String,
    source: String,
    message: String,
    /// JSON of the tags map (not aliased `tags`: the alias would shadow the column in WHERE).
    tags_json: String,
    attributes: String,
    trace_id: String,
    span_id: String,
}

/// Empty columns mean "not set".
pub(crate) fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

impl From<LogRow> for LogEvent {
    fn from(r: LogRow) -> Self {
        LogEvent {
            id: r.id,
            timestamp: r.ts,
            status: r.status,
            service: r.service,
            host: r.host,
            source: r.source,
            message: r.message,
            tags: serde_json::from_str(&r.tags_json).unwrap_or_default(),
            attributes: serde_json::from_str(&r.attributes).unwrap_or(serde_json::Value::Null),
            trace_id: non_empty(r.trace_id),
            span_id: non_empty(r.span_id),
        }
    }
}

/// WHERE clause scoping a query to the org and range, plus the search terms, and the table they
/// run against.
struct Scope {
    clause: String,
    params: Vec<Param>,
    table: &'static str,
}

/// `index:rehydrated` searches logs loaded back from archives instead of the live index.
fn index_table(terms: &[Term]) -> &'static str {
    let rehydrated = terms.iter().any(|t| matches!(t, Term::Tag { key, values, negate: false } if key == "index" && values.iter().any(|v| v == "rehydrated")));
    if rehydrated {
        "logs_rehydrated"
    } else {
        "logs"
    }
}

fn scope(org: &str, range: TimeRange, terms: &[Term], skip_key: Option<&str>) -> Scope {
    let table = index_table(terms);
    // `index:rehydrated` picks the table; it isn't an index_name filter.
    let filtered: Vec<Term> = terms.iter().filter(|t| !(table == "logs_rehydrated" && matches!(t, Term::Tag { key, .. } if key == "index"))).cloned().collect();
    let f = build_where(&LogsSchema, &filtered, skip_key);
    let mut params = vec![Param::Str(org.to_string()), Param::Int(range.from), Param::Int(range.to)];
    params.extend(f.params);
    Scope {
        clause: format!("org_id = ? AND timestamp >= fromUnixTimestamp64Milli(?) AND timestamp <= fromUnixTimestamp64Milli(?) AND ({})", f.clause),
        params,
        table,
    }
}

/// Binds params in order onto a query built with `?` placeholders.
pub(crate) fn bind(mut q: clickhouse::query::Query, params: &[Param]) -> clickhouse::query::Query {
    for p in params {
        q = match p {
            Param::Str(s) => q.bind(s.as_str()),
            Param::Int(i) => q.bind(*i),
        };
    }
    q
}

// ---------------------------------------------------------------- search

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogSearchRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// Page size (max 1000). Default 100.
    pub limit: Option<u64>,
    /// Opaque cursor from a previous page's `nextCursor`.
    pub cursor: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogSearchPage {
    pub logs: Vec<LogEvent>,
    /// Matching logs in the range (not just this page).
    pub total: u64,
    pub next_cursor: Option<String>,
}

#[derive(clickhouse::Row, Deserialize)]
struct Count {
    n: u64,
}

/// Newest-first page of logs matching the query.
#[utoipa::path(post, path = "/api/v1/logs/search", request_body = LogSearchRequest,
    responses((status = 200, body = LogSearchPage), (status = 400, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogSearchRequest>) -> ApiResult<LogSearchPage> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let limit = req.limit.unwrap_or(100).clamp(1, MAX_LIMIT);
    let s = scope(&org, req.range, &terms, None);
    let table = s.table;

    // Keyset pagination on (timestamp, id): cursor = "<ts_ms>:<uuid>".
    let mut page_clause = s.clause.clone();
    let mut page_params = s.params.clone();
    if let Some(c) = &req.cursor {
        let (ts, id) = c.split_once(':').ok_or_else(|| ApiError::BadRequest("invalid cursor".into()))?;
        let ts: i64 = ts.parse().map_err(|_| ApiError::BadRequest("invalid cursor".into()))?;
        page_clause.push_str(" AND (timestamp, id) < (fromUnixTimestamp64Milli(?), toUUID(?))");
        page_params.push(Param::Int(ts));
        page_params.push(Param::Str(id.to_string()));
    }
    page_params.push(Param::Int(limit as i64));

    let rows_q = bind(
        st.ch(&org).query(&format!(
            "SELECT toString(id) AS id, toUnixTimestamp64Milli(timestamp) AS ts, status, service, host, source, message, \
             toJSONString(tags) AS tags_json, attributes, trace_id, span_id FROM {table} WHERE {page_clause} ORDER BY timestamp DESC, id DESC LIMIT ?"
        )),
        &page_params,
    );
    let count_q = bind(st.ch(&org).query(&format!("SELECT count() AS n FROM {table} WHERE {}", s.clause)), &s.params);

    let (rows, count) = tokio::try_join!(rows_q.fetch_all::<LogRow>(), count_q.fetch_one::<Count>())?;
    let next_cursor = if rows.len() as u64 == limit { rows.last().map(|r| format!("{}:{}", r.ts, r.id)) } else { None };
    Ok(Json(LogSearchPage { logs: rows.into_iter().map(Into::into).collect(), total: count.n, next_cursor }))
}

// ---------------------------------------------------------------- histogram

#[derive(Deserialize, ToSchema)]
pub struct LogQueryRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema, Default, Clone)]
pub struct StatusCounts {
    pub error: u64,
    pub warn: u64,
    pub info: u64,
    pub debug: u64,
}

#[derive(Serialize, ToSchema)]
pub struct HistogramBucket {
    /// Bucket start, epoch milliseconds.
    pub t: i64,
    pub counts: StatusCounts,
}

#[derive(clickhouse::Row, Deserialize)]
struct BucketRow {
    t: i64,
    status: String,
    n: u64,
}

/// ~60 buckets across the range, like the reference histogram.
fn bucket_ms(range: TimeRange) -> i64 {
    let target = (range.to - range.from) / 60;
    [5_000, 10_000, 15_000, 30_000, 60_000, 300_000, 900_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000]
        .into_iter()
        .find(|s| *s >= target)
        .unwrap_or(43_200_000)
}

/// Status-stacked log counts over time.
#[utoipa::path(post, path = "/api/v1/logs/histogram", request_body = LogQueryRequest,
    responses((status = 200, body = Vec<HistogramBucket>)), tag = "logs")]
pub async fn histogram(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogQueryRequest>) -> ApiResult<Vec<HistogramBucket>> {
    req.range.validate()?;
    let size = bucket_ms(req.range);
    let s = scope(&org, req.range, &parse_query(&req.query), None);
    let table = s.table;
    let mut params = vec![Param::Int(size / 1000)];
    params.extend(s.params);
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT toUnixTimestamp(toStartOfInterval(timestamp, toIntervalSecond(?))) * 1000 AS t, toString(status) AS status, count() AS n \
             FROM {table} WHERE {} GROUP BY t, status ORDER BY t",
            s.clause
        )),
        &params,
    )
    .fetch_all::<BucketRow>()
    .await?;

    let start = req.range.from / size * size;
    let mut buckets: Vec<HistogramBucket> = (0..=((req.range.to - start) / size))
        .map(|i| HistogramBucket { t: start + i * size, counts: StatusCounts::default() })
        .collect();
    for r in rows {
        if let Some(b) = buckets.get_mut(((r.t - start) / size) as usize) {
            match r.status.as_str() {
                "error" => b.counts.error += r.n,
                "warn" => b.counts.warn += r.n,
                "debug" => b.counts.debug += r.n,
                _ => b.counts.info += r.n,
            }
        }
    }
    Ok(Json(buckets))
}

// ---------------------------------------------------------------- facets

#[derive(Serialize, ToSchema)]
pub struct FacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
pub struct Facet {
    pub key: String,
    pub label: String,
    pub group: String,
    pub values: Vec<FacetValue>,
}

#[derive(clickhouse::Row, Deserialize)]
struct ValueRow {
    v: String,
    n: u64,
}

const FACETS: &[(&str, &str, &str)] = &[
    ("source", "Source", "CORE"),
    ("host", "Host", "CORE"),
    ("service", "Service", "CORE"),
    ("status", "Status", "CORE"),
    ("env", "Env", "ENVIRONMENT"),
    ("kube_cluster_name", "Cluster", "ENVIRONMENT"),
    ("region", "Region", "ENVIRONMENT"),
    ("kube_namespace", "Namespace", "KUBERNETES"),
    ("kube_deployment", "Deployment", "KUBERNETES"),
    ("pod_name", "Pod Name", "KUBERNETES"),
    ("container_name", "Container Name", "KUBERNETES"),
    ("@http.method", "Method", "HTTP"),
    ("@http.status_code", "Status Code", "HTTP"),
];

/// Top values of `key`; `skip_own` drops the key's own filter (facets keep showing unchecked values).
#[allow(clippy::too_many_arguments)]
async fn values_for(st: &AppState, org: &str, range: TimeRange, terms: &[Term], key: &str, filter: Option<&str>, limit: i64, skip_own: bool) -> Result<Vec<ValueRow>, ApiError> {
    let (expr, key_param) = LogsSchema.field(key);
    let s = if skip_own { scope(org, range, terms, Some(key)) } else { top_scope(org, range, terms) };
    let table = s.table;
    let mut params = Vec::new();
    params.extend(key_param.clone()); // SELECT expr
    params.extend(s.params);
    params.extend(key_param); // non-empty check
    let mut extra = String::new();
    if let Some(f) = filter {
        extra = " AND positionCaseInsensitive(v, ?) > 0".to_string();
        params.push(Param::Str(f.to_string()));
    }
    params.push(Param::Int(limit));
    Ok(bind(
        st.ch(org).query(&format!(
            "SELECT toString({expr}) AS v, count() AS n FROM {table} WHERE {} AND {expr} != ''{extra} GROUP BY v ORDER BY n DESC LIMIT ?",
            s.clause
        )),
        &params,
    )
    .fetch_all::<ValueRow>()
    .await?)
}

#[derive(Deserialize, ToSchema)]
pub struct LogFacetsRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// Facets the viewer added (`kube_node`, `@http.url`), up to 20, after the built-in ones.
    #[serde(default)]
    pub extra: Vec<String>,
}

/// The added facets worth counting: new, plausible keys, at most 20.
fn extra_facets(extra: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in extra {
        let valid = !k.is_empty() && k.len() <= 200 && !k.chars().any(char::is_whitespace);
        if valid && !FACETS.iter().any(|(f, _, _)| f == k) && !out.contains(k) && out.len() < 20 {
            out.push(k.clone());
        }
    }
    out
}

/// Top values per facet. Each facet ignores its own filter so unchecked values keep their counts.
#[utoipa::path(post, path = "/api/v1/logs/facets", request_body = LogFacetsRequest,
    responses((status = 200, body = Vec<Facet>)), tag = "logs")]
pub async fn facets(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogFacetsRequest>) -> ApiResult<Vec<Facet>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let extra = extra_facets(&req.extra);
    let (results, indexes, added) = tokio::try_join!(
        try_join_all(FACETS.iter().map(|(key, _, _)| values_for(&st, &org, req.range, &terms, key, None, 10, true))),
        values_for(&st, &org, req.range, &terms, "index", None, 50, true),
        try_join_all(extra.iter().map(|key| values_for(&st, &org, req.range, &terms, key, None, 10, true))),
    )?;
    let mut out = vec![Facet { key: "index".into(), label: "Index".into(), group: "CORE".into(), values: indexes.into_iter().map(|r| FacetValue { value: r.v, count: r.n }).collect() }];
    for ((key, label, group), rows) in FACETS.iter().zip(results) {
        let mut values: Vec<FacetValue> = rows.into_iter().map(|r| FacetValue { value: r.v, count: r.n }).collect();
        if *key == "status" {
            for s in ["error", "warn", "info", "debug"] {
                if !values.iter().any(|v| v.value == s) {
                    values.push(FacetValue { value: s.into(), count: 0 });
                }
            }
            let order = |s: &str| ["error", "warn", "info", "debug"].iter().position(|x| *x == s).unwrap_or(9);
            values.sort_by_key(|v| order(&v.value));
        }
        out.push(Facet { key: (*key).into(), label: (*label).into(), group: (*group).into(), values });
    }
    for (key, rows) in extra.into_iter().zip(added) {
        out.push(Facet { label: key.trim_start_matches('@').into(), key, group: "ADDED".into(), values: rows.into_iter().map(|r| FacetValue { value: r.v, count: r.n }).collect() });
    }
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
pub struct LogField {
    /// A tag (`kube_node`) or an attribute (`@http.url`).
    pub key: String,
    /// Logs of the sample that have it.
    pub count: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct FieldRow {
    key: String,
    n: u64,
}

/// The tags and attributes of the matching logs (from the newest 10,000), to add as facets.
#[utoipa::path(post, path = "/api/v1/logs/fields", request_body = LogQueryRequest,
    responses((status = 200, body = Vec<LogField>)), tag = "logs")]
pub async fn fields(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogQueryRequest>) -> ApiResult<Vec<LogField>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let s = scope(&org, req.range, &terms, None);
    let mut params = s.params;
    params.push(Param::Int(PATTERN_SAMPLE));
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT key, count() AS n FROM (SELECT arrayJoin(arrayConcat(mapKeys(tags), arrayMap(k -> concat('@', k), mapKeys(attr)))) AS key \
             FROM (SELECT tags, attr FROM {} WHERE {} ORDER BY timestamp DESC LIMIT ?)) GROUP BY key ORDER BY n DESC, key LIMIT 300",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all::<FieldRow>()
    .await?;
    Ok(Json(rows.into_iter().map(|r| LogField { key: r.key, count: r.n }).collect()))
}

// ---------------------------------------------------------------- top values

#[derive(Deserialize, ToSchema)]
pub struct LogTopRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// The key to count logs by: `source`, `service`, a tag (`kube_namespace`) or an attribute (`@http.status_code`).
    pub key: String,
    /// How many values, 1–100 (default 10).
    pub limit: Option<i64>,
}

/// The scope of top values: every filter applies, the counted key's own included.
fn top_scope(org: &str, range: TimeRange, terms: &[Term]) -> Scope {
    scope(org, range, terms, None)
}

fn top_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(10).clamp(1, 100)
}

/// The most frequent values of a key among the matching logs (dashboard toplists and sunbursts).
#[utoipa::path(post, path = "/api/v1/logs/top", request_body = LogTopRequest,
    responses((status = 200, body = Vec<FacetValue>)), tag = "logs")]
pub async fn top(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogTopRequest>) -> ApiResult<Vec<FacetValue>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let rows = values_for(&st, &org, req.range, &terms, &req.key, None, top_limit(req.limit), false).await?;
    Ok(Json(rows.into_iter().map(|r| FacetValue { value: r.v, count: r.n }).collect()))
}

// ---------------------------------------------------------------- patterns

/// Characters that separate the parts of a word (`/api/v1/orders/42`, `key=value`, `a:b`).
const PART_DELIMITERS: &str = "/:=,;()[]{}\"'?&@#|<>";

/// Whether a part of a word changes from one log to the next: numbers, hex, UUIDs, long ids.
fn variable_part(p: &str) -> bool {
    let hex = |c: char| c.is_ascii_hexdigit() || c == '-';
    let digits = p.chars().any(|c| c.is_ascii_digit());
    p.starts_with(|c: char| c.is_ascii_digit())
        || (p.starts_with("0x") && p.len() > 2)
        || (digits && p.len() >= 8 && p.chars().all(hex))
        || (digits && p.len() >= 16)
}

/// A message's pattern: its variable parts become `*` (Log Explorer → Group into Patterns).
fn pattern_of(message: &str) -> String {
    // Without ANSI colors (`ESC[94m`).
    let mut text = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            text.push(c);
        }
    }
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            let mut parts: Vec<(String, bool)> = Vec::new(); // (text, is a delimiter)
            let mut current = String::new();
            for c in word.chars() {
                if PART_DELIMITERS.contains(c) {
                    if !current.is_empty() {
                        parts.push((std::mem::take(&mut current), false));
                    }
                    parts.push((c.to_string(), true));
                } else {
                    current.push(c);
                }
            }
            if !current.is_empty() {
                parts.push((current, false));
            }
            let values: Vec<&String> = parts.iter().filter(|(_, d)| !d).map(|(p, _)| p).collect();
            if values.is_empty() || !values.iter().all(|p| variable_part(p)) {
                return parts.iter().map(|(p, d)| if !d && variable_part(p) { "*" } else { p.as_str() }).collect();
            }
            // One number with a short unit keeps the unit (`12.5ms` → `*ms`); timestamps, IPs,
            // UUIDs and other all-variable words become a single `*`.
            if let [only] = values.as_slice() {
                let unit = only.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
                if (1..=3).contains(&unit.len()) && unit.chars().all(|c| c.is_ascii_alphabetic()) && parts.len() == 1 {
                    return format!("*{unit}");
                }
            }
            "*".to_string()
        })
        .collect();
    words.join(" ")
}

/// Logs grouped into patterns: the newest this many matching logs are read.
const PATTERN_SAMPLE: i64 = 10_000;

#[derive(clickhouse::Row, Deserialize)]
struct PatternRow {
    message: String,
    status: String,
    service: String,
}

#[derive(Serialize, ToSchema)]
pub struct LogPattern {
    /// The message with its variable parts as `*`.
    pub pattern: String,
    /// Logs of the sample with this pattern.
    pub count: u64,
    /// The most common status and service among them.
    pub status: String,
    pub service: String,
    /// The newest message with this pattern.
    pub sample: String,
}

fn group_patterns(rows: Vec<PatternRow>, limit: usize) -> Vec<LogPattern> {
    struct Group {
        count: u64,
        sample: String,
        statuses: HashMap<String, u64>,
        services: HashMap<String, u64>,
    }
    let most = |m: &HashMap<String, u64>| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(k, _)| k.clone()).unwrap_or_default();
    let mut groups: HashMap<String, Group> = HashMap::new();
    for r in rows {
        let g = groups.entry(pattern_of(&r.message)).or_insert_with(|| Group { count: 0, sample: r.message.clone(), statuses: HashMap::new(), services: HashMap::new() });
        g.count += 1;
        *g.statuses.entry(r.status).or_default() += 1;
        *g.services.entry(r.service).or_default() += 1;
    }
    let mut out: Vec<LogPattern> =
        groups.into_iter().map(|(pattern, g)| LogPattern { status: most(&g.statuses), service: most(&g.services), pattern, count: g.count, sample: g.sample }).collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.pattern.cmp(&b.pattern)));
    out.truncate(limit);
    out
}

#[derive(Deserialize, ToSchema)]
pub struct LogPatternsRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// How many patterns, 1–200 (default 50).
    pub limit: Option<usize>,
}

/// The matching logs grouped by pattern (Group into Patterns), from the newest 10,000 of them.
#[utoipa::path(post, path = "/api/v1/logs/patterns", request_body = LogPatternsRequest,
    responses((status = 200, body = Vec<LogPattern>)), tag = "logs")]
pub async fn patterns(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogPatternsRequest>) -> ApiResult<Vec<LogPattern>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let s = scope(&org, req.range, &terms, None);
    let mut params = s.params;
    params.push(Param::Int(PATTERN_SAMPLE));
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT substring(message, 1, 300) AS message, toString(status) AS status, toString(service) AS service FROM {} WHERE {} ORDER BY timestamp DESC LIMIT ?",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all::<PatternRow>()
    .await?;
    Ok(Json(group_patterns(rows, req.limit.unwrap_or(50).clamp(1, 200))))
}

// ---------------------------------------------------------------- suggestions

#[derive(Deserialize, ToSchema)]
pub struct SuggestRequest {
    pub range: TimeRange,
    /// The term being typed, e.g. `serv`, `service:`, `service:pho`, `-env:`.
    #[serde(default)]
    pub partial: String,
}

#[derive(Serialize, ToSchema)]
pub struct QuerySuggestion {
    pub text: String,
    pub count: u64,
}

const SUGGEST_KEYS: &[&str] = &["service", "status", "host", "source", "env", "kube_namespace", "pod_name", "container_name", "workloadname"];

/// Autocomplete for the query bar: keys (`service:`) and `key:value` pairs with counts.
#[utoipa::path(post, path = "/api/v1/logs/suggest", request_body = SuggestRequest,
    responses((status = 200, body = Vec<QuerySuggestion>)), tag = "logs")]
pub async fn suggest(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<SuggestRequest>) -> ApiResult<Vec<QuerySuggestion>> {
    req.range.validate()?;
    let (neg, p) = match req.partial.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", req.partial.as_str()),
    };
    if let Some((key, value)) = p.split_once(':') {
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-@".contains(c)) {
            return Ok(Json(vec![]));
        }
        let rows = values_for(&st, &org, req.range, &[], key, (!value.is_empty()).then_some(value), 6, true).await?;
        return Ok(Json(rows.into_iter().map(|r| QuerySuggestion { text: format!("{neg}{key}:{}", r.v), count: r.n }).collect()));
    }
    let lower = p.to_lowercase();
    let keys: Vec<QuerySuggestion> = SUGGEST_KEYS
        .iter()
        .filter(|k| k.starts_with(&lower))
        .take(if lower.is_empty() { 5 } else { 3 })
        .map(|k| QuerySuggestion { text: format!("{neg}{k}:"), count: 0 })
        .collect();
    if lower.is_empty() {
        return Ok(Json(keys));
    }
    // Values containing the text, across the common keys.
    let per_key = try_join_all(["service", "host", "env", "kube_namespace"].iter().map(|k| values_for(&st, &org, req.range, &[], k, Some(&lower), 3, true))).await?;
    let mut pairs: Vec<QuerySuggestion> = ["service", "host", "env", "kube_namespace"]
        .iter()
        .zip(per_key)
        .flat_map(|(k, rows)| rows.into_iter().map(move |r| QuerySuggestion { text: format!("{neg}{k}:{}", r.v), count: r.n }))
        .collect();
    pairs.sort_by(|a, b| b.count.cmp(&a.count));
    let mut out = keys;
    out.extend(pairs.into_iter().take(6));
    Ok(Json(out))
}

// ---------------------------------------------------------------- insights

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogInsight {
    pub kind: String,
    pub attribute: String,
    pub value: String,
    pub share_of_errors: f64,
    pub share_of_logs: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct ServiceErrors {
    service: String,
    errors: u64,
    total: u64,
}

/// Error outliers: services whose share of errors far exceeds their share of traffic.
#[utoipa::path(post, path = "/api/v1/logs/insights", request_body = LogQueryRequest,
    responses((status = 200, body = Vec<LogInsight>)), tag = "logs")]
pub async fn insights(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LogQueryRequest>) -> ApiResult<Vec<LogInsight>> {
    req.range.validate()?;
    let s = scope(&org, req.range, &[], None);
    let table = s.table;
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT toString(service) AS service, countIf(status = 'error') AS errors, count() AS total FROM {table} WHERE {} GROUP BY service",
            s.clause
        )),
        &s.params,
    )
    .fetch_all::<ServiceErrors>()
    .await?;
    let total_errors: u64 = rows.iter().map(|r| r.errors).sum();
    let total: u64 = rows.iter().map(|r| r.total).sum();
    if total_errors < 3 || total == 0 {
        return Ok(Json(vec![]));
    }
    let mut out: Vec<LogInsight> = rows
        .into_iter()
        .filter(|r| r.errors > 0)
        .map(|r| LogInsight {
            kind: "error_outlier".into(),
            attribute: "service".into(),
            value: r.service,
            share_of_errors: r.errors as f64 / total_errors as f64,
            share_of_logs: r.total as f64 / total as f64,
        })
        .filter(|i| i.share_of_errors > 2.0 * i.share_of_logs)
        .collect();
    out.sort_by(|a, b| b.share_of_errors.total_cmp(&a.share_of_errors));
    out.truncate(3);
    Ok(Json(out))
}

// ---------------------------------------------------------------- log monitors

/// `logs("service:checkout status:error") by {service,host}` → (search query, group-by keys).
pub(crate) fn parse_log_monitor(q: &str) -> Result<(String, Vec<String>), String> {
    let q = q.trim();
    let rest = q.strip_prefix("logs(\"").ok_or("log monitor queries look like logs(\"<search>\") by {<tags>}")?;
    let mut search = String::new();
    let mut chars = rest.char_indices();
    let mut end = None;
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, n)) = chars.next() {
                    search.push(n);
                }
            }
            '"' => {
                end = Some(i);
                break;
            }
            c => search.push(c),
        }
    }
    let end = end.ok_or("unterminated search string")?;
    let tail = rest[end + 1..].trim_start().strip_prefix(')').ok_or("expected ')' after the search")?.trim();
    let by = if tail.is_empty() {
        Vec::new()
    } else {
        let inner = tail.strip_prefix("by").map(str::trim).and_then(|t| t.strip_prefix('{')).and_then(|t| t.strip_suffix('}')).ok_or("expected `by {tag,…}`")?;
        inner.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    if by.len() > 4 || by.iter().any(|k| k.len() > 100 || !k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '@' | '/'))) {
        return Err("group by at most 4 tags or @attributes".into());
    }
    Ok((search, by))
}

#[derive(clickhouse::Row, Deserialize)]
struct CountRow {
    g: Vec<String>,
    ts: i64,
    n: u64,
}

/// Matching logs per `step_ms`, per group (log monitors). Intervals without logs count as 0, so
/// "fewer than N logs" monitors work and a quiet service reads 0 rather than no data.
pub(crate) async fn log_counts(st: &AppState, org: &str, search: &str, by: &[String], range: TimeRange, step_ms: i64) -> Result<Vec<super::metrics::Group>, ApiError> {
    let terms = parse_query(search);
    let s = scope(org, range, &terms, None);
    let table = s.table;
    let mut params = Vec::new();
    let mut exprs = Vec::new();
    for k in by {
        let (e, p) = LogsSchema.field(k);
        exprs.push(format!("toString({e})"));
        params.extend(p);
    }
    let g = if exprs.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", exprs.join(", ")) };
    let step_s = (step_ms / 1000).max(1);
    params.extend(s.params);
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT {g} AS g, toInt64(toUnixTimestamp(toStartOfInterval(timestamp, INTERVAL {step_s} SECOND))) * 1000 AS ts, count() AS n \
             FROM {table} WHERE {} GROUP BY g, ts ORDER BY g, ts",
            s.clause
        )),
        &params,
    )
    .fetch_all::<CountRow>()
    .await?;
    let start = range.from - range.from.rem_euclid(step_s * 1000);
    let grid: Vec<i64> = (0..).map(|i| start + i * step_s * 1000).take_while(|t| *t <= range.to).collect();
    let mut by_group: std::collections::BTreeMap<Vec<String>, HashMap<i64, f64>> = std::collections::BTreeMap::new();
    for r in rows {
        by_group.entry(r.g).or_default().insert(r.ts, r.n as f64);
    }
    if by.is_empty() && by_group.is_empty() {
        by_group.insert(Vec::new(), HashMap::new());
    }
    Ok(by_group
        .into_iter()
        .map(|(g, counts)| super::metrics::Group {
            tags: by.iter().cloned().zip(g.into_iter().map(|v| if v.is_empty() { "N/A".into() } else { v })).collect(),
            points: grid.iter().map(|t| (*t, counts.get(t).copied().unwrap_or(0.0))).collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_replace_the_variable_parts_of_a_message() {
        assert_eq!(pattern_of("GET /api/v1/orders/48213 200 in 12.5ms"), "GET /api/v1/orders/* * in *ms");
        assert_eq!(pattern_of("user 3f2a9c1e-77b4-4d0e-9a51-0c6f5e2b8d14 logged in from 10.2.1.23"), "user * logged in from *");
        assert_eq!(pattern_of("\u{1b}[94mGET\u{1b}[0m /metrics took 0x7ffe3a10"), "GET /metrics took *");
        assert_eq!(pattern_of("2026-10-27T19:54:21.008Z worker 7 done"), "* worker * done");
        // Same shape, same pattern.
        assert_eq!(pattern_of("job 1 failed after 3 retries"), pattern_of("job 982 failed after 11 retries"));
    }

    #[test]
    fn patterns_group_logs_and_keep_their_most_common_status_and_service() {
        let row = |m: &str, status: &str, service: &str| PatternRow { message: m.into(), status: status.into(), service: service.into() };
        let rows = vec![
            row("job 1 failed", "error", "billing"),
            row("job 2 failed", "error", "billing"),
            row("job 3 failed", "warn", "billing"),
            row("cache warm", "info", "catalog-service"),
        ];
        let p = group_patterns(rows, 10);
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].pattern.as_str(), p[0].count, p[0].status.as_str(), p[0].service.as_str()), ("job * failed", 3, "error", "billing"));
        assert_eq!(p[0].sample, "job 1 failed");
        assert_eq!(p[1].pattern, "cache warm");
        assert_eq!(group_patterns(vec![row("a 1", "info", "s"), row("b", "info", "s")], 1).len(), 1);
    }

    #[test]
    fn added_facets_are_new_valid_keys_at_most_twenty() {
        let keys = |v: &[&str]| v.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        assert_eq!(extra_facets(&keys(&["kube_node", "service", "@http.url", "kube_node", "", "a b"])), keys(&["kube_node", "@http.url"]));
        assert_eq!(extra_facets(&(0..30).map(|i| format!("k{i}")).collect::<Vec<_>>()).len(), 20);
    }

    #[test]
    fn top_values_count_with_the_keys_own_filter() {
        let range = TimeRange { from: 0, to: 1 };
        let terms = parse_query("source:(etcd OR coredns) env:prod");
        // Facets drop their own key's filter; top values keep it.
        assert!(top_scope("o", range, &terms).params.len() > scope("o", range, &terms, Some("source")).params.len());
        assert_eq!(top_limit(None), 10);
        assert_eq!(top_limit(Some(0)), 1);
        assert_eq!(top_limit(Some(5000)), 100);
    }

    #[test]
    fn log_monitor_queries() {
        assert_eq!(parse_log_monitor(r#"logs("service:a status:error")"#).unwrap(), ("service:a status:error".into(), vec![]));
        assert_eq!(parse_log_monitor(r#"logs("\"bad response\"") by {service, host}"#).unwrap(), ("\"bad response\"".into(), vec!["service".into(), "host".into()]));
        assert!(parse_log_monitor("avg:system.load.1{*}").is_err());
        assert!(parse_log_monitor(r#"logs("x") by {a'b}"#).is_err());
    }
}
