//! Logs endpoints backed by `oc.logs` in ClickHouse. Shapes mirror `web/src/api/types.ts`.

use std::collections::{HashMap, HashSet};

use axum::extract::State;
use axum::Json;
use futures::future::try_join_all;
use oc_query::{build_where, parse_query, LogsSchema, Param, Schema, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::routes::log_aggregates::{self, Bound, Plan, LOG_FACETS};
use crate::routes::standard_attributes::org_paths;
use crate::state::{AppState, OrgId};

const MAX_RANGE_MS: i64 = 31 * 24 * 3600 * 1000;
const MAX_LIMIT: u64 = 1000;

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq)]
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

/// WHERE clause scoping a query to the org, the reader's log access (restriction queries) and the
/// range, plus the search terms, and the table they run against.
pub(crate) struct Scope {
    pub(crate) clause: String,
    pub(crate) params: Vec<Param>,
    pub(crate) table: &'static str,
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

/// Every read of the log tables builds its WHERE clause here, so none can skip the restriction.
pub(crate) fn scope(org: &str, access: &LogAccess, range: TimeRange, terms: &[Term], skip_key: Option<&str>) -> Scope {
    let table = index_table(terms);
    // `index:rehydrated` picks the table; it isn't an index_name filter.
    let filtered: Vec<Term> = terms.iter().filter(|t| !(table == "logs_rehydrated" && matches!(t, Term::Tag { key, .. } if key == "index"))).cloned().collect();
    let f = build_where(&LogsSchema, &filtered, skip_key);
    let mut params = vec![Param::Str(org.to_string()), Param::Int(range.from), Param::Int(range.to)];
    params.extend(f.params);
    let mut clause = format!("org_id = ? AND timestamp >= fromUnixTimestamp64Milli(?) AND timestamp <= fromUnixTimestamp64Milli(?) AND ({})", f.clause);
    // The restriction is built apart from the search: `skip_key` never drops it.
    if let Some(r) = access.clickhouse_filter() {
        clause.push_str(&format!(" AND ({})", r.clause));
        params.extend(r.params);
    }
    Scope { clause, params, table }
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
    /// The reader sees only the logs their restriction queries allow (Logs → Configuration → Data Access).
    pub restricted: bool,
}

#[derive(clickhouse::Row, Deserialize)]
struct Count {
    n: u64,
}

/// Newest-first page of logs matching the query.
#[utoipa::path(post, path = "/api/v1/logs/search", request_body = LogSearchRequest,
    responses((status = 200, body = LogSearchPage), (status = 400, body = crate::error::ErrorBody)), tag = "logs")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogSearchRequest>) -> ApiResult<LogSearchPage> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let limit = req.limit.unwrap_or(100).clamp(1, MAX_LIMIT);
    let s = scope(&org, &access, req.range, &terms, None);
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
    Ok(Json(LogSearchPage { logs: rows.into_iter().map(Into::into).collect(), total: count.n, next_cursor, restricted: access.restricted() }))
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
pub async fn histogram(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogQueryRequest>) -> ApiResult<Vec<HistogramBucket>> {
    req.range.validate()?;
    let size = bucket_ms(req.range);
    let s = scope(&org, &access, req.range, &parse_query(&req.query), None);
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
    /// The counts are estimated from the newest logs of the range (too many logs to count them all).
    #[serde(default)]
    pub approximate: bool,
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

/// A plan to count the logs by `key`: from the facet aggregate when it can, else from the logs.
/// `skip_own` drops the key's own filter (facets keep showing unchecked values).
fn key_plan(org: &str, access: &LogAccess, range: TimeRange, terms: &[Term], key: &str, skip_own: bool, since: Option<i64>) -> Plan {
    log_aggregates::plan(org, access, &LOG_FACETS, range, terms, skip_own.then_some(key), since, &[key])
}

/// The SQL of the top `limit` values of `key` in `plan` (containing `filter`, when set).
fn values_query(plan: &Plan, key: &str, filter: Option<&str>, limit: i64) -> (String, Vec<Param>) {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    if let (Some(agg), Some(column)) = (&plan.agg, LOG_FACETS.column(key)) {
        parts.push(format!("SELECT toString({column}) AS v, sum(n) AS c FROM {} WHERE {} GROUP BY v", agg.table, agg.clause));
        params.extend(agg.params.iter().cloned());
    }
    let (expr, key_param) = LogsSchema.field(key);
    params.extend(key_param.clone()); // SELECT expr
    params.extend(plan.logs.params.iter().cloned());
    params.extend(key_param); // non-empty check
    parts.push(format!("SELECT toString({expr}) AS v, count() AS c FROM {} WHERE {} AND {expr} != '' GROUP BY v", plan.logs.table, plan.logs.clause));
    let mut extra = String::new();
    if let Some(f) = filter {
        extra = " AND positionCaseInsensitive(v, ?) > 0".to_string();
        params.push(Param::Str(f.to_string()));
    }
    params.push(Param::Int(limit));
    (format!("SELECT v, sum(c) AS n FROM ({}) WHERE v != ''{extra} GROUP BY v ORDER BY n DESC LIMIT ?", parts.join(" UNION ALL ")), params)
}

/// Top values of `key` in `plan`.
async fn values_for(st: &AppState, org: &str, plan: &Plan, key: &str, filter: Option<&str>, limit: i64) -> Result<Vec<ValueRow>, ApiError> {
    let (sql, params) = values_query(plan, key, filter, limit);
    Ok(bind(st.ch(org).query(&sql), &params).fetch_all::<ValueRow>().await?)
}

/// Facet counts read from the logs (not the facet aggregate) read at most about this many: the
/// newest logs of the range when it has more, with estimated counts.
const FACET_SCAN_ROWS: u64 = 1_000_000;

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

/// The group of added facets that are standard attributes of the org.
const STANDARD_GROUP: &str = "STANDARD ATTRIBUTES";

/// Whether a facet key is one of the org's standard attributes (`@http.url`; tags aren't).
fn is_standard(key: &str, standard: &HashSet<String>) -> bool {
    key.strip_prefix('@').is_some_and(|path| standard.contains(path))
}

/// Added facets are grouped apart when they are standard attributes.
fn added_group(key: &str, standard: &HashSet<String>) -> &'static str {
    if is_standard(key, standard) {
        STANDARD_GROUP
    } else {
        "ADDED"
    }
}

/// The facets whose key no filter of the search uses: their counts share one scan.
fn shared_facet_keys<'a>(keys: &[&'a str], terms: &[Term]) -> Vec<&'a str> {
    keys.iter().copied().filter(|k| !terms.iter().any(|t| matches!(t, Term::Tag { key, .. } if key == k))).collect()
}

#[derive(clickhouse::Row, Deserialize)]
struct KeyValueRow {
    k: String,
    v: String,
    n: u64,
}

/// One scan for the top `limit` values of every key: each matching log becomes (key, value) pairs.
/// The aggregate's part of `plan` is used when it keeps every key.
fn shared_facets_query(keys: &[&str], plan: &Plan, limit: i64) -> (String, Vec<Param>) {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    let columns: Option<Vec<&str>> = keys.iter().map(|k| LOG_FACETS.column(k)).collect();
    if let (Some(agg), Some(columns)) = (&plan.agg, columns) {
        // The window's minutes collapse first, then each row becomes (key, value) pairs.
        let pairs: Vec<String> = columns.iter().map(|c| format!("(?, toString({c}))")).collect();
        params.extend(agg.params.iter().cloned());
        params.extend(keys.iter().map(|k| Param::Str((*k).to_string())));
        parts.push(format!(
            "SELECT kv.1 AS k, kv.2 AS v, sum(c) AS c FROM (SELECT {cols}, sum(n) AS c FROM {} WHERE {} GROUP BY {cols}) ARRAY JOIN [{}] AS kv GROUP BY k, v",
            agg.table,
            agg.clause,
            pairs.join(", "),
            cols = columns.join(", ")
        ));
    }
    // The logs' value combinations are counted first, then each becomes (key, value) pairs.
    let mut fields = Vec::new();
    let mut pairs = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        let (expr, key_param) = LogsSchema.field(key);
        params.extend(key_param);
        fields.push(format!("toString({expr}) AS f{i}"));
        pairs.push(format!("(?, f{i})"));
    }
    params.extend(plan.logs.params.iter().cloned());
    params.extend(keys.iter().map(|k| Param::Str((*k).to_string())));
    let names: Vec<String> = (0..keys.len()).map(|i| format!("f{i}")).collect();
    parts.push(format!(
        "SELECT kv.1 AS k, kv.2 AS v, sum(c) AS c FROM (SELECT {}, count() AS c FROM {} WHERE {} GROUP BY {}) ARRAY JOIN [{}] AS kv GROUP BY k, v",
        fields.join(", "),
        plan.logs.table,
        plan.logs.clause,
        names.join(", "),
        pairs.join(", ")
    ));
    params.push(Param::Int(limit));
    let sql = format!("SELECT k, v, sum(c) AS n FROM ({}) WHERE v != '' GROUP BY k, v ORDER BY k, n DESC LIMIT ? BY k", parts.join(" UNION ALL "));
    (sql, params)
}

/// Top values of the keys in one scan of `plan`.
async fn shared_values(st: &AppState, org: &str, keys: &[&str], plan: &Plan, limit: i64) -> Result<Vec<KeyValueRow>, ApiError> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let (sql, params) = shared_facets_query(keys, plan, limit);
    Ok(bind(st.ch(org).query(&sql), &params).fetch_all::<KeyValueRow>().await?)
}

/// Top values per facet. Each facet ignores its own filter so unchecked values keep their counts.
/// Standard facets are counted from the facet aggregate when the search and the reader's
/// restriction allow; other counts read the newest logs of the range, about a million at most.
#[utoipa::path(post, path = "/api/v1/logs/facets", request_body = LogFacetsRequest,
    responses((status = 200, body = Vec<Facet>)), tag = "logs")]
pub async fn facets(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogFacetsRequest>) -> ApiResult<Vec<Facet>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let extra = extra_facets(&req.extra);
    let keys: Vec<&str> = FACETS.iter().map(|(k, _, _)| *k).chain(extra.iter().map(String::as_str)).collect();
    let shared = shared_facet_keys(&keys, &terms);
    let own: Vec<&str> = keys.iter().copied().filter(|k| !shared.contains(k)).chain(["index"]).collect();
    let ch = st.ch(&org);
    let since = log_aggregates::since(&ch, &org, &LOG_FACETS).await?;
    // Which counts the aggregate answers: the shared scan's kept keys, and each own-filter facet.
    let shared_plan = log_aggregates::plan(&org, &access, &LOG_FACETS, req.range, &terms, None, since, &[]);
    let (exact_shared, scanned_shared): (Vec<&str>, Vec<&str>) = shared.iter().partition(|k| shared_plan.agg.is_some() && LOG_FACETS.column(k).is_some());
    let own_plans: Vec<Plan> = own.iter().map(|k| key_plan(&org, &access, req.range, &terms, k, true, since)).collect();
    let bound = if !scanned_shared.is_empty() || own_plans.iter().any(|p| p.agg.is_none()) {
        log_aggregates::bound_logs(&ch, &org, req.range, since, FACET_SCAN_ROWS).await?
    } else {
        Bound { range: req.range, factor: 1.0 }
    };
    // The scanned counts: the bound's range, from the logs only.
    let scanned_plan = log_aggregates::plan(&org, &access, &LOG_FACETS, bound.range, &terms, None, None, &[]);
    let own_plans: Vec<(Plan, bool)> =
        own.iter().zip(own_plans).map(|(k, p)| if p.agg.is_some() { (p, false) } else { (key_plan(&org, &access, bound.range, &terms, k, true, None), bound.approximate()) }).collect();
    let standard = async { Ok::<_, ApiError>(if extra.is_empty() { HashSet::new() } else { org_paths(&st, &org).await? }) };
    let (exact_rows, scanned_rows, own_rows, standard) = tokio::try_join!(
        shared_values(&st, &org, &exact_shared, &shared_plan, 10),
        shared_values(&st, &org, &scanned_shared, &scanned_plan, 10),
        try_join_all(own.iter().zip(&own_plans).map(|(key, (p, _))| values_for(&st, &org, p, key, None, if *key == "index" { 50 } else { 10 }))),
        standard,
    )?;
    // Counts per key, and whether they are estimated.
    let mut by_key: HashMap<String, (Vec<ValueRow>, bool)> = HashMap::new();
    for r in exact_rows {
        by_key.entry(r.k).or_default().0.push(ValueRow { v: r.v, n: r.n });
    }
    for k in &scanned_shared {
        by_key.insert((*k).to_string(), (Vec::new(), bound.approximate()));
    }
    for r in scanned_rows {
        by_key.entry(r.k).or_default().0.push(ValueRow { v: r.v, n: bound.scale(r.n) });
    }
    for ((key, (_, estimated)), rows) in own.iter().zip(&own_plans).zip(own_rows) {
        let rows = if *estimated { rows.into_iter().map(|r| ValueRow { v: r.v, n: bound.scale(r.n) }).collect() } else { rows };
        by_key.insert((*key).to_string(), (rows, *estimated));
    }
    let facet = |key: &str, label: &str, group: &str, by_key: &mut HashMap<String, (Vec<ValueRow>, bool)>| {
        let (rows, approximate) = by_key.remove(key).unwrap_or_default();
        Facet { key: key.into(), label: label.into(), group: group.into(), values: rows.into_iter().map(|r| FacetValue { value: r.v, count: r.n }).collect(), approximate }
    };
    let mut out = vec![facet("index", "Index", "CORE", &mut by_key)];
    for (key, label, group) in FACETS {
        let mut f = facet(key, label, group, &mut by_key);
        if *key == "status" {
            for s in ["error", "warn", "info", "debug"] {
                if !f.values.iter().any(|v| v.value == s) {
                    f.values.push(FacetValue { value: s.into(), count: 0 });
                }
            }
            let order = |s: &str| ["error", "warn", "info", "debug"].iter().position(|x| *x == s).unwrap_or(9);
            f.values.sort_by_key(|v| order(&v.value));
        }
        out.push(f);
    }
    for key in extra {
        out.push(facet(&key, key.trim_start_matches('@'), added_group(&key, &standard), &mut by_key));
    }
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
pub struct LogField {
    /// A tag (`kube_node`) or an attribute (`@http.url`).
    pub key: String,
    /// Logs of the sample that have it.
    pub count: u64,
    /// One of the org's standard attributes (Logs → Configuration → Standard Attributes).
    pub standard: bool,
}

#[derive(clickhouse::Row, Deserialize)]
struct FieldRow {
    key: String,
    n: u64,
}

/// The tags and attributes of the matching logs (from the newest 10,000), to add as facets.
#[utoipa::path(post, path = "/api/v1/logs/fields", request_body = LogQueryRequest,
    responses((status = 200, body = Vec<LogField>)), tag = "logs")]
pub async fn fields(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogQueryRequest>) -> ApiResult<Vec<LogField>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let s = scope(&org, &access, req.range, &terms, None);
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
    let standard = org_paths(&st, &org).await?;
    Ok(Json(rows.into_iter().map(|r| LogField { standard: is_standard(&r.key, &standard), key: r.key, count: r.n }).collect()))
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

fn top_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(10).clamp(1, 100)
}

/// The most frequent values of a key among the matching logs (dashboard toplists and sunbursts).
#[utoipa::path(post, path = "/api/v1/logs/top", request_body = LogTopRequest,
    responses((status = 200, body = Vec<FacetValue>)), tag = "logs")]
pub async fn top(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogTopRequest>) -> ApiResult<Vec<FacetValue>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let since = log_aggregates::since(&st.ch(&org), &org, &LOG_FACETS).await?;
    let rows = values_for(&st, &org, &key_plan(&org, &access, req.range, &terms, &req.key, false, since), &req.key, None, top_limit(req.limit)).await?;
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
pub(crate) fn pattern_of(message: &str) -> String {
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
pub async fn patterns(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogPatternsRequest>) -> ApiResult<Vec<LogPattern>> {
    req.range.validate()?;
    let terms = parse_query(&req.query);
    let s = scope(&org, &access, req.range, &terms, None);
    let mut params = s.params;
    params.push(Param::Int(PATTERN_SAMPLE));
    let rows = bind(
        st.ch(&org).query(&format!(
            "SELECT substringUTF8(message, 1, 300) AS message, toString(status) AS status, toString(service) AS service FROM {} WHERE {} ORDER BY timestamp DESC LIMIT ?",
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
pub async fn suggest(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<SuggestRequest>) -> ApiResult<Vec<QuerySuggestion>> {
    req.range.validate()?;
    let (neg, p) = match req.partial.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", req.partial.as_str()),
    };
    if let Some((key, value)) = p.split_once(':') {
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-@".contains(c)) {
            return Ok(Json(vec![]));
        }
        let since = log_aggregates::since(&st.ch(&org), &org, &LOG_FACETS).await?;
        let rows = values_for(&st, &org, &key_plan(&org, &access, req.range, &[], key, true, since), key, (!value.is_empty()).then_some(value), 6).await?;
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
    let since = log_aggregates::since(&st.ch(&org), &org, &LOG_FACETS).await?;
    let plans: Vec<Plan> = ["service", "host", "env", "kube_namespace"].iter().map(|k| key_plan(&org, &access, req.range, &[], k, true, since)).collect();
    let per_key = try_join_all(["service", "host", "env", "kube_namespace"].iter().zip(&plans).map(|(k, p)| values_for(&st, &org, p, k, Some(&lower), 3))).await?;
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
pub async fn insights(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<LogQueryRequest>) -> ApiResult<Vec<LogInsight>> {
    req.range.validate()?;
    let s = scope(&org, &access, req.range, &[], None);
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
pub(crate) async fn log_counts(st: &AppState, org: &str, access: &LogAccess, search: &str, by: &[String], range: TimeRange, step_ms: i64) -> Result<Vec<super::metrics::Group>, ApiError> {
    let terms = parse_query(search);
    let s = scope(org, access, range, &terms, None);
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
    fn standard_attributes_are_marked() {
        let standard: HashSet<String> = ["http.url".to_string(), "usr.id".to_string()].into();
        assert!(is_standard("@http.url", &standard));
        assert!(!is_standard("http.url", &standard), "a tag isn't an attribute");
        assert!(!is_standard("@team", &standard));
        assert_eq!(added_group("@usr.id", &standard), STANDARD_GROUP);
        assert_eq!(added_group("kube_node", &standard), "ADDED");
    }

    #[test]
    fn facets_without_their_own_filter_share_one_scan() {
        let terms = parse_query("service:billing env:prod");
        let keys = ["source", "service", "kube_namespace", "@http.method"];
        // `service` filters itself, so it keeps its own query; the others share one.
        assert_eq!(shared_facet_keys(&keys, &terms), vec!["source", "kube_namespace", "@http.method"]);
        let plan = log_aggregates::plan("o", &LogAccess::org_background(), &LOG_FACETS, TimeRange { from: 0, to: 1 }, &terms, None, None, &[]);
        let scope_params = plan.logs.params.len();
        let (sql, params) = shared_facets_query(&["source", "kube_namespace", "@http.method"], &plan, 10);
        assert!(sql.contains("ARRAY JOIN") && sql.contains("LIMIT ? BY k"), "{sql}");
        // A label per key, a field parameter per tag or attribute, the scope, the limit.
        assert_eq!(params.len(), 3 + 2 + scope_params + 1);
    }

    #[test]
    fn standard_facets_count_from_the_aggregate_and_the_logs_around_it() {
        const T0: i64 = 1_793_095_200_000;
        let range = TimeRange { from: T0 + 5_000, to: T0 + 30 * 60_000 };
        let all = LogAccess::org_background();
        let plan = log_aggregates::plan("o", &all, &LOG_FACETS, range, &parse_query("env:prod"), None, Some(T0), &[]);
        let (agg_params, logs_params) = (plan.agg.as_ref().unwrap().params.len(), plan.logs.params.len());
        let (sql, params) = shared_facets_query(&["source", "kube_namespace"], &plan, 10);
        assert!(sql.contains("FROM oc.log_facets_1m WHERE ") && sql.contains(" UNION ALL "), "{sql}");
        // The aggregate's scope and labels, then labels, a tag parameter and the logs' scope, then the limit.
        assert_eq!(params.len(), agg_params + 2 + 2 + 1 + logs_params + 1);
        assert_eq!(params[agg_params], Param::Str("source".into()));
        // A key it doesn't keep: the logs only.
        let (sql, _) = shared_facets_query(&["source", "pod_name"], &plan, 10);
        assert!(!sql.contains("log_facets_1m"), "{sql}");
        let (sql, params) = values_query(&key_plan("o", &all, range, &parse_query("service:billing"), "service", true, Some(T0)), "service", Some("bill"), 10);
        assert!(sql.starts_with("SELECT v, sum(c) AS n FROM (SELECT toString(service) AS v, sum(n) AS c FROM oc.log_facets_1m WHERE "), "{sql}");
        assert_eq!(&params[params.len() - 2..], &[Param::Str("bill".into()), Param::Int(10)]);
        assert!(!values_query(&key_plan("o", &all, range, &[], "pod_name", true, Some(T0)), "pod_name", None, 10).0.contains("log_facets_1m"));
    }

    /// Standard facet counts from the facet aggregate (and the logs around it) are those counted from
    /// the logs alone. Skipped unless OC_TEST_CLICKHOUSE_URL points at a ClickHouse server (user
    /// `default`, or OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).
    #[tokio::test]
    async fn facet_counts_from_the_aggregate_match_the_logs() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let ch = live.client.clone().with_database("oc");
        let org = format!("facet-parity-{}", std::process::id());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let t0 = now / 60_000 * 60_000 - 30 * 60_000;
        ch.query(&format!(
            "INSERT INTO oc.logs (org_id, timestamp, status, service, host, source, message, tags, attr, retention_days) \
             SELECT '{org}', fromUnixTimestamp64Milli(toInt64({t0} + number * 89)), ['info', 'warn', 'error', 'debug', 'info'][number % 5 + 1], \
                    ['billing', 'checkout', 'order-service', 'catalog-service'][number % 4 + 1], concat('i-0a1b2c3d4e5f6000', toString(number % 3)), \
                    ['python', 'go'][number % 2 + 1], concat('request ', toString(number)), \
                    map('env', if(number % 6 = 0, 'staging', 'prod'), 'kube_namespace', ['payments', 'storefront'][number % 2 + 1], 'pod_name', concat('pod-', toString(number % 7))), \
                    map('http.method', ['GET', 'POST'][number % 2 + 1]), [15, 3][number % 2 + 1] \
             FROM numbers(8000)"
        ))
        .execute()
        .await
        .expect("insert logs");
        let rows = |sql: String, params: Vec<Param>| {
            let ch = ch.clone();
            async move {
                let mut rows: Vec<(String, String, u64)> =
                    bind(ch.query(&sql), &params).fetch_all::<KeyValueRow>().await.unwrap_or_else(|e| panic!("{sql}: {e}")).into_iter().map(|r| (r.k, r.v, r.n)).collect();
                rows.sort();
                rows
            }
        };
        let values = |sql: String, params: Vec<Param>| {
            let ch = ch.clone();
            async move {
                let mut rows: Vec<(String, u64)> = bind(ch.query(&sql), &params).fetch_all::<ValueRow>().await.unwrap_or_else(|e| panic!("{sql}: {e}")).into_iter().map(|r| (r.v, r.n)).collect();
                rows.sort();
                rows
            }
        };
        let since = log_aggregates::since(&ch, &org, &LOG_FACETS).await.unwrap();
        assert_eq!(since, Some(t0 + 60_000));
        let range = TimeRange { from: t0 + 30_250, to: t0 + 11 * 60_000 + 40_500 };
        let keys = ["source", "host", "service", "status", "env", "kube_namespace"];
        let mut used = 0;
        for (query, access) in [
            ("", LogAccess::org_background()),
            ("env:prod -status:debug", LogAccess::org_background()),
            ("service:(billing OR order-*)", LogAccess::org_background()),
            ("", restricted_to("kube_namespace:payments")),
            ("pod_name:pod-1", LogAccess::org_background()),
            ("", restricted_to("@http.method:GET")),
        ] {
            let terms = parse_query(query);
            let plan = log_aggregates::plan(&org, &access, &LOG_FACETS, range, &terms, None, since, &[]);
            used += usize::from(plan.agg.is_some());
            // Today's query, over the logs.
            let mut params = Vec::new();
            let mut pairs = Vec::new();
            for key in keys {
                let (expr, key_param) = LogsSchema.field(key);
                params.push(Param::Str(key.to_string()));
                params.extend(key_param);
                pairs.push(format!("(?, toString({expr}))"));
            }
            let s = scope(&org, &access, range, &terms, None);
            params.extend(s.params.clone());
            params.push(Param::Int(100));
            let reference = format!(
                "SELECT kv.1 AS k, kv.2 AS v, count() AS n FROM (SELECT [{}] AS kvs FROM {} WHERE {}) ARRAY JOIN kvs AS kv \
                 WHERE v != '' GROUP BY k, v ORDER BY k, n DESC LIMIT ? BY k",
                pairs.join(", "),
                s.table,
                s.clause
            );
            let expected = rows(reference, params).await;
            assert!(!expected.is_empty(), "{query}: no fixtures");
            let (sql, params) = shared_facets_query(&keys, &plan, 100);
            assert_eq!(rows(sql, params).await, expected, "facets of {query:?} (aggregate: {})", plan.agg.is_some());
            // One facet without its own filter.
            let s = scope(&org, &access, range, &terms, Some("service"));
            let mut params = s.params.clone();
            params.push(Param::Int(10));
            let reference = format!("SELECT toString(service) AS v, count() AS n FROM {} WHERE {} AND service != '' GROUP BY v ORDER BY n DESC LIMIT ?", s.table, s.clause);
            let (sql, own) = values_query(&key_plan(&org, &access, range, &terms, "service", true, since), "service", None, 10);
            assert_eq!(values(sql, own).await, values(reference, params).await, "services of {query:?}");
        }
        assert_eq!(used, 4);
    }

    #[test]
    fn top_values_count_with_the_keys_own_filter() {
        let range = TimeRange { from: 0, to: 1 };
        let terms = parse_query("source:(etcd OR coredns) env:prod");
        // Facets drop their own key's filter; top values keep it.
        let all = LogAccess::org_background();
        assert!(key_plan("o", &all, range, &terms, "source", false, None).logs.params.len() > key_plan("o", &all, range, &terms, "source", true, None).logs.params.len());
        assert_eq!(top_limit(None), 10);
        assert_eq!(top_limit(Some(0)), 1);
        assert_eq!(top_limit(Some(5000)), 100);
    }

    fn restricted_to(query: &str) -> LogAccess {
        let rule = crate::log_access::Restriction { terms: parse_query(query), roles: ["r-finance".to_string()].into(), teams: HashSet::new() };
        LogAccess::resolve(&[rule], false, &["r-finance".to_string()], &[])
    }

    #[test]
    fn the_scope_always_applies_the_readers_restriction() {
        let range = TimeRange { from: 0, to: 1 };
        let all = scope("o", &LogAccess::org_background(), range, &parse_query("env:prod"), None);
        let only = scope("o", &restricted_to("service:billing"), range, &parse_query("env:prod"), None);
        assert!(only.clause.ends_with(" AND (((service = ?)))"), "{}", only.clause);
        assert_eq!(only.params.len(), all.params.len() + 1);
        assert_eq!(only.params.last(), Some(&Param::Str("billing".into())));
        // Facets drop the search's own filter on a key, never the restriction's.
        let facet = scope("o", &restricted_to("service:billing"), range, &parse_query("service:order-service"), Some("service"));
        assert_eq!(facet.params.last(), Some(&Param::Str("billing".into())));
        // Rehydrated logs too.
        let rehydrated = scope("o", &restricted_to("service:billing"), range, &parse_query("index:rehydrated"), None);
        assert_eq!((rehydrated.table, rehydrated.params.last()), ("logs_rehydrated", Some(&Param::Str("billing".into()))));
        // Nothing assigned: nothing matches.
        let none = LogAccess::resolve(&[crate::log_access::Restriction { terms: parse_query("service:billing"), roles: HashSet::new(), teams: HashSet::new() }], false, &[], &[]);
        assert!(scope("o", &none, range, &[], None).clause.ends_with(" AND (0)"));
    }

    #[test]
    fn log_monitor_queries() {
        assert_eq!(parse_log_monitor(r#"logs("service:a status:error")"#).unwrap(), ("service:a status:error".into(), vec![]));
        assert_eq!(parse_log_monitor(r#"logs("\"bad response\"") by {service, host}"#).unwrap(), ("\"bad response\"".into(), vec!["service".into(), "host".into()]));
        assert!(parse_log_monitor("avg:system.load.1{*}").is_err());
        assert!(parse_log_monitor(r#"logs("x") by {a'b}"#).is_err());
    }
}
