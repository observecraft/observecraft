//! Synthetic Monitoring & Testing → Results Explorer and Test Coverage.
//!
//! - **Results Explorer** searches the stored runs of every test of the org (Postgres
//!   `synthetic_results`, kept 30 days) with the explorer syntax: `key:value`, `key:(a OR b)`,
//!   `-key:value`, numbers compared with `>`, `>=`, `<`, `<=`, `*` wildcards, free words on the
//!   test name and the error. Keys (`@` optional): `type` (always `result`), `status`
//!   (passed, failed), `test.name`, `test.id`, `test.type`, `location`, `http.status_code`,
//!   `result.retries`, `result.duration` (ms), `trigger`, `env`, `team`; other keys match the test's
//!   `key:value` tags. It returns the runs, their count, a pass/fail series, per-test groups and facets.
//! - **Test Coverage** lists the HTTP endpoints APM saw serving traffic (entry spans with an
//!   `http.route`, or a `METHOD /path` span name) and the HTTP tests whose method and URL path hit
//!   each one.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// A bound value of a results filter.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterArg {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// A SQL condition over `synthetic_results r JOIN synthetic_tests t` with `$n` placeholders.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultFilter {
    pub sql: String,
    pub args: Vec<FilterArg>,
}

/// Splits on whitespace, keeping quoted strings and parenthesised groups together.
fn tokenize(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut depth, mut quoted) = (String::new(), 0usize, false);
    for ch in q.chars() {
        match ch {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && !quoted && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// `%`, `_` and `\` are literal in LIKE patterns built from user text.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// `*` wildcards become `%`; without one the value is contained anywhere.
fn like_pattern(v: &str) -> String {
    if v.contains('*') {
        v.split('*').map(like_escape).collect::<Vec<_>>().join("%")
    } else {
        format!("%{}%", like_escape(v))
    }
}

/// `>=400` → (">=", "400"); `400` → ("=", "400").
fn comparison(v: &str) -> (&'static str, &str) {
    for op in [">=", "<=", ">", "<"] {
        if let Some(rest) = v.strip_prefix(op) {
            return (op, rest.trim());
        }
    }
    ("=", v)
}

struct Builder {
    next: usize,
    args: Vec<FilterArg>,
}

impl Builder {
    fn arg(&mut self, a: FilterArg) -> String {
        self.args.push(a);
        let p = format!("${}", self.next);
        self.next += 1;
        p
    }

    /// One `key:value` condition.
    fn condition(&mut self, key: &str, value: &str) -> Result<String, String> {
        let key = key.trim_start_matches('@');
        let num = |v: &str| v.parse::<f64>().map_err(|_| format!("'{v}' is not a number ({key})"));
        let int = |v: &str| v.parse::<i64>().map_err(|_| format!("'{v}' is not a whole number ({key})"));
        Ok(match key {
            "type" => (if value == "result" { "TRUE" } else { "FALSE" }).to_string(),
            "status" | "result.status" => {
                let passed = match value.to_ascii_lowercase().as_str() {
                    "passed" | "pass" | "ok" => true,
                    "failed" | "fail" | "error" => false,
                    _ => return Err(format!("status is passed or failed, not '{value}'")),
                };
                format!("r.passed = {}", self.arg(FilterArg::Bool(passed)))
            }
            "test.name" | "test" => format!("t.name ILIKE {}", self.arg(FilterArg::Text(like_pattern(value)))),
            "test.id" | "test_id" => format!("r.test_id::text = {}", self.arg(FilterArg::Text(value.to_ascii_lowercase()))),
            "test.type" | "test_type" | "subtype" => format!("t.type = {}", self.arg(FilterArg::Text(value.to_ascii_lowercase()))),
            "location" => format!("r.location = {}", self.arg(FilterArg::Text(value.to_string()))),
            "trigger" | "result.trigger" => format!("r.trigger = {}", self.arg(FilterArg::Text(value.to_ascii_lowercase()))),
            "env" => format!("t.env = {}", self.arg(FilterArg::Text(value.to_string()))),
            "team" => format!("t.team = {}", self.arg(FilterArg::Text(value.to_string()))),
            "http.status_code" | "http_status" | "status_code" => {
                if value.contains('*') {
                    format!("r.status_code::text LIKE {}", self.arg(FilterArg::Text(like_pattern(value))))
                } else {
                    let (op, v) = comparison(value);
                    format!("r.status_code {op} {}", self.arg(FilterArg::Int(int(v)?)))
                }
            }
            "result.retries" | "retries" => {
                let (op, v) = comparison(value);
                format!("(r.attempts - 1) {op} {}", self.arg(FilterArg::Int(int(v)?)))
            }
            "result.duration" | "duration" => {
                let (op, v) = comparison(value);
                format!("r.duration_ms {op} {}", self.arg(FilterArg::Float(num(v)?)))
            }
            _ => format!("{} = ANY(t.tags)", self.arg(FilterArg::Text(format!("{key}:{value}")))),
        })
    }
}

/// The WHERE fragment for an explorer query; placeholders are numbered from `first`.
pub fn result_filter(query: &str, first: usize) -> Result<ResultFilter, String> {
    let mut b = Builder { next: first, args: Vec::new() };
    let mut parts: Vec<String> = Vec::new();
    for token in tokenize(query) {
        if token == "AND" {
            continue;
        }
        let (negate, term) = match token.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => (true, rest.to_string()),
            _ => (false, token.clone()),
        };
        let keyed = term.split_once(':').filter(|(k, v)| !k.is_empty() && !v.is_empty() && k.chars().all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '_' | '-')));
        let sql = match keyed {
            Some((key, raw)) => {
                let values: Vec<String> = if raw.starts_with('(') && raw.ends_with(')') && raw.len() >= 2 {
                    raw[1..raw.len() - 1].split(" OR ").map(unquote).filter(|v| !v.is_empty()).collect()
                } else {
                    vec![unquote(raw)]
                };
                let mut conds = Vec::new();
                for v in &values {
                    conds.push(b.condition(key, v)?);
                }
                if conds.len() == 1 {
                    conds.remove(0)
                } else {
                    format!("({})", conds.join(" OR "))
                }
            }
            None => {
                let p = b.arg(FilterArg::Text(like_pattern(&unquote(&term))));
                format!("(t.name ILIKE {p} OR r.error ILIKE {p})")
            }
        };
        parts.push(if negate { format!("NOT COALESCE(({sql}), false)") } else { sql });
    }
    // A lone always-true term needs no clause.
    parts.retain(|p| p != "TRUE");
    let sql = if parts.is_empty() { "TRUE".to_string() } else { parts.join(" AND ") };
    Ok(ResultFilter { sql, args: b.args })
}

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// An endpoint (method, route) from an entry span: its `http.route` and method attributes, or a
/// `METHOD /path` span name.
pub fn endpoint_of(span_name: &str, route: &str, method: &str) -> Option<(String, String)> {
    let method = method.trim().to_ascii_uppercase();
    if route.starts_with('/') {
        return Some((method, route.trim().to_string()));
    }
    let (m, path) = span_name.trim().split_once(' ')?;
    let path = path.trim();
    (METHODS.contains(&m) && path.starts_with('/')).then(|| (m.to_string(), path.to_string()))
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// A URL path served by a route template (`/orders/{id}`, `/orders/:id`, `/orders/?`, `/static/*`).
pub fn route_matches(route: &str, path: &str) -> bool {
    let (r, p) = (segments(route), segments(path));
    for (i, seg) in r.iter().enumerate() {
        let rest = *seg == "*" || *seg == "**" || seg.starts_with("{*");
        if rest && i == r.len() - 1 {
            return p.len() >= i;
        }
        let Some(actual) = p.get(i) else { return false };
        let param = seg.starts_with('{') || seg.starts_with(':') || seg.starts_with('<') || *seg == "?" || *seg == "*";
        if !param && seg != actual {
            return false;
        }
    }
    r.len() == p.len()
}

/// The method and path an HTTP test calls (variables in the host don't matter).
pub fn test_target(method: &str, url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(&super::synthetics_settings::neutralized(url.trim())).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    Some((method.trim().to_ascii_uppercase(), parsed.path().to_string()))
}

// ================================================================ results explorer API

#[derive(Deserialize, ToSchema)]
pub struct SyntheticResultSearchRequest {
    /// Explorer syntax (see the module docs), e.g. `status:failed @http.status_code:>=500`.
    #[serde(default)]
    pub query: String,
    pub range: TimeRange,
    #[serde(default)]
    pub offset: i64,
    /// 1–500 (default 50).
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticResultRow {
    pub id: i64,
    pub test_id: String,
    pub test_name: String,
    pub test_type: String,
    pub location: String,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub duration_ms: f64,
    pub passed: bool,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub attempts: i32,
    /// scheduled | manual
    pub trigger: String,
    pub env: String,
    pub team: String,
    pub tags: Vec<String>,
    pub failed_assertions: i64,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct SyntheticResultBucket {
    /// Bucket start, epoch milliseconds.
    pub t: i64,
    pub passed: i64,
    pub failed: i64,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticResultGroup {
    pub test_id: String,
    pub test_name: String,
    pub test_type: String,
    pub runs: i64,
    pub failed: i64,
    pub avg_duration_ms: f64,
    pub p95_duration_ms: f64,
    pub last_run_at: i64,
}

#[derive(Serialize, ToSchema)]
pub struct SyntheticFacetValue {
    pub value: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema)]
pub struct SyntheticResultFacet {
    /// The query key a value filters on.
    pub key: String,
    pub label: String,
    pub values: Vec<SyntheticFacetValue>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticResultSearchPage {
    /// Runs matching the query.
    pub total: i64,
    /// Newest first.
    pub results: Vec<SyntheticResultRow>,
    pub series: Vec<SyntheticResultBucket>,
    pub bucket_ms: i64,
    /// Per test, most runs first (at most 100).
    pub groups: Vec<SyntheticResultGroup>,
    pub facets: Vec<SyntheticResultFacet>,
}

/// Facets: (query key, label, SQL expression).
const FACETS: [(&str, &str, &str); 8] = [
    ("status", "Status", "CASE WHEN r.passed THEN 'passed' ELSE 'failed' END"),
    ("@test.type", "Test Type", "t.type"),
    ("@test.name", "Test Name", "t.name"),
    ("location", "Location", "r.location"),
    ("@http.status_code", "HTTP Status Code", "COALESCE(r.status_code::text, '')"),
    ("@trigger", "Trigger", "r.trigger"),
    ("env", "Env", "t.env"),
    ("team", "Team", "t.team"),
];

fn bind_all<'q, O>(mut q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>, args: &'q [FilterArg]) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    for a in args {
        q = match a {
            FilterArg::Text(s) => q.bind(s),
            FilterArg::Int(i) => q.bind(i),
            FilterArg::Float(f) => q.bind(f),
            FilterArg::Bool(b) => q.bind(b),
        };
    }
    q
}

fn bucket_ms(range: &TimeRange) -> i64 {
    let target = (range.to - range.from) / 60;
    [60_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000].into_iter().find(|s| *s >= target).unwrap_or(86_400_000)
}

/// Searches the runs of every test of the org.
#[utoipa::path(post, path = "/api/v1/synthetics/results/search", operation_id = "search_synthetic_results", request_body = SyntheticResultSearchRequest,
    responses((status = 200, body = SyntheticResultSearchPage), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn search_results(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SyntheticResultSearchRequest>) -> ApiResult<SyntheticResultSearchPage> {
    req.range.validate()?;
    let limit = req.limit.unwrap_or(50).clamp(1, 500);
    let offset = req.offset.clamp(0, 100_000);
    // $1 org, $2 from, $3 to; the query's arguments follow.
    let filter = result_filter(&req.query, 4).map_err(ApiError::BadRequest)?;
    let from_where = format!(
        "FROM synthetic_results r JOIN synthetic_tests t ON t.id = r.test_id AND t.org_id = r.org_id \
         WHERE r.org_id = $1 AND r.started_at >= to_timestamp($2) AND r.started_at <= to_timestamp($3) AND ({})",
        filter.sql
    );
    let (from, to) = (req.range.from as f64 / 1000.0, req.range.to as f64 / 1000.0);
    let n = filter.args.len();
    let (total,): (i64,) = bind_all(sqlx::query_as(&format!("SELECT count(*) {from_where}")).bind(&ctx.org_id).bind(from).bind(to), &filter.args).fetch_one(&st.postgres).await?;
    let results: Vec<SyntheticResultRow> = bind_all(
        sqlx::query_as(&format!(
            "SELECT r.id, r.test_id::text AS test_id, t.name AS test_name, t.type AS test_type, r.location, (extract(epoch FROM r.started_at) * 1000)::bigint AS started_at, \
             r.duration_ms, r.passed, r.status_code, r.error, r.attempts, r.trigger, t.env, t.team, t.tags, \
             (SELECT count(*) FROM jsonb_array_elements(r.details->'assertions') a WHERE (a->>'passed') = 'false') AS failed_assertions \
             {from_where} ORDER BY r.started_at DESC, r.id DESC LIMIT ${} OFFSET ${}",
            n + 4,
            n + 5
        ))
        .bind(&ctx.org_id)
        .bind(from)
        .bind(to),
        &filter.args,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&st.postgres)
    .await?;
    let step = bucket_ms(&req.range);
    let series: Vec<SyntheticResultBucket> = bind_all(
        sqlx::query_as(&format!(
            "SELECT (floor(extract(epoch FROM r.started_at) * 1000 / ${0}) * ${0})::bigint AS t, count(*) FILTER (WHERE r.passed) AS passed, count(*) FILTER (WHERE NOT r.passed) AS failed \
             {from_where} GROUP BY 1 ORDER BY 1",
            n + 4
        ))
        .bind(&ctx.org_id)
        .bind(from)
        .bind(to),
        &filter.args,
    )
    .bind(step as f64)
    .fetch_all(&st.postgres)
    .await?;
    let groups: Vec<SyntheticResultGroup> = bind_all(
        sqlx::query_as(&format!(
            "SELECT r.test_id::text AS test_id, t.name AS test_name, t.type AS test_type, count(*) AS runs, count(*) FILTER (WHERE NOT r.passed) AS failed, \
             avg(r.duration_ms) AS avg_duration_ms, percentile_cont(0.95) WITHIN GROUP (ORDER BY r.duration_ms) AS p95_duration_ms, \
             (extract(epoch FROM max(r.started_at)) * 1000)::bigint AS last_run_at {from_where} GROUP BY r.test_id, t.name, t.type ORDER BY runs DESC, t.name LIMIT 100"
        ))
        .bind(&ctx.org_id)
        .bind(from)
        .bind(to),
        &filter.args,
    )
    .fetch_all(&st.postgres)
    .await?;
    let mut facets = Vec::new();
    for (key, label, expr) in FACETS {
        let rows: Vec<(String, i64)> =
            bind_all(sqlx::query_as(&format!("SELECT {expr} AS v, count(*) {from_where} GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 20")).bind(&ctx.org_id).bind(from).bind(to), &filter.args)
                .fetch_all(&st.postgres)
                .await?;
        let values = rows.into_iter().filter(|(v, _)| !v.is_empty()).map(|(value, count)| SyntheticFacetValue { value, count }).collect();
        facets.push(SyntheticResultFacet { key: key.into(), label: label.into(), values });
    }
    Ok(Json(SyntheticResultSearchPage { total, results, series, bucket_ms: step, groups, facets }))
}

// ================================================================ coverage API

#[derive(Deserialize, ToSchema)]
pub struct SyntheticCoverageRequest {
    pub range: TimeRange,
    pub env: Option<String>,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct SyntheticCoveringTest {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticEndpoint {
    pub service: String,
    /// Empty when the spans don't say.
    pub method: String,
    pub route: String,
    pub requests: u64,
    pub errors: u64,
    /// 95th percentile latency in milliseconds (the slowest of the span names serving the endpoint).
    pub p95_ms: f64,
    /// HTTP tests calling this method and path.
    pub tests: Vec<SyntheticCoveringTest>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticCoverage {
    /// Most requests first.
    pub endpoints: Vec<SyntheticEndpoint>,
    pub http_tests: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct EndpointAgg {
    service: String,
    route: String,
    method: String,
    name: String,
    hits: u64,
    errors: u64,
    p95_ms: f64,
}

/// HTTP endpoints APM saw serving traffic, and the HTTP tests covering each.
#[utoipa::path(post, path = "/api/v1/synthetics/coverage", operation_id = "get_synthetic_coverage", request_body = SyntheticCoverageRequest,
    responses((status = 200, body = SyntheticCoverage), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn coverage(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SyntheticCoverageRequest>) -> ApiResult<SyntheticCoverage> {
    req.range.validate()?;
    let env = req.env.clone().filter(|e| !e.is_empty() && e != "*");
    let rows = st
        .ch(&ctx.org_id)
        .query(&format!(
            "SELECT toString(ServiceName) AS service, toString(SpanAttributes['http.route']) AS route, \
             toString(if(SpanAttributes['http.request.method'] != '', SpanAttributes['http.request.method'], SpanAttributes['http.method'])) AS method, \
             toString(SpanName) AS name, count() AS hits, countIf(StatusCode = 'Error') AS errors, toFloat64(quantile(0.95)(Duration)) / 1e6 AS p95_ms FROM otel.otel_traces \
             WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND SpanKind = 'Server' \
             AND (SpanAttributes['http.route'] != '' OR match(SpanName, '^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS) /')) {} \
             GROUP BY service, route, method, name ORDER BY hits DESC LIMIT 2000",
            if env.is_some() { "AND ResourceAttributes['deployment.environment'] = ?" } else { "" }
        ))
        .bind(req.range.from)
        .bind(req.range.to);
    let rows = match &env {
        Some(e) => rows.bind(e),
        None => rows,
    };
    let rows: Vec<EndpointAgg> = rows.fetch_all().await?;
    let tests: Vec<(String, String, serde_json::Value)> =
        sqlx::query_as("SELECT id::text, name, request FROM synthetic_tests WHERE org_id = $1 AND type = 'http' ORDER BY lower(name)").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let targets: Vec<(SyntheticCoveringTest, String, String)> = tests
        .iter()
        .filter_map(|(id, name, request)| {
            let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
            let url = request.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            test_target(method, url).map(|(m, p)| (SyntheticCoveringTest { id: id.clone(), name: name.clone() }, m, p))
        })
        .collect();
    let mut merged: BTreeMap<(String, String, String), (u64, u64, f64)> = BTreeMap::new();
    for r in rows {
        if let Some((method, route)) = endpoint_of(&r.name, &r.route, &r.method) {
            let e = merged.entry((r.service, method, route)).or_default();
            e.0 += r.hits;
            e.1 += r.errors;
            e.2 = e.2.max(r.p95_ms);
        }
    }
    let mut endpoints: Vec<SyntheticEndpoint> = merged
        .into_iter()
        .map(|((service, method, route), (requests, errors, p95_ms))| {
            let tests = targets.iter().filter(|(_, m, p)| (method.is_empty() || *m == method) && route_matches(&route, p)).map(|(t, _, _)| t.clone()).collect();
            SyntheticEndpoint { service, method, route, requests, errors, p95_ms, tests }
        })
        .collect();
    endpoints.sort_by(|a, b| b.requests.cmp(&a.requests).then_with(|| a.route.cmp(&b.route)));
    Ok(Json(SyntheticCoverage { endpoints, http_tests: tests.len() as i64 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use FilterArg::*;

    fn f(q: &str) -> ResultFilter {
        result_filter(q, 4).unwrap()
    }

    #[test]
    fn an_empty_query_matches_every_result() {
        assert_eq!(f(""), ResultFilter { sql: "TRUE".into(), args: vec![] });
        assert_eq!(f("@type:result"), ResultFilter { sql: "TRUE".into(), args: vec![] });
        assert_eq!(f("type:batch").sql, "FALSE");
    }

    #[test]
    fn keys_become_bound_conditions() {
        assert_eq!(f("status:failed"), ResultFilter { sql: "r.passed = $4".into(), args: vec![Bool(false)] });
        assert_eq!(f("@result.status:passed").args, vec![Bool(true)]);
        assert_eq!(
            f("@test.type:http location:observecraft-server"),
            ResultFilter { sql: "t.type = $4 AND r.location = $5".into(), args: vec![Text("http".into()), Text("observecraft-server".into())] }
        );
        assert_eq!(f("@test.name:Checkout*"), ResultFilter { sql: "t.name ILIKE $4".into(), args: vec![Text("Checkout%".into())] });
        assert_eq!(f("@test.name:\"Checkout API\"").args, vec![Text("%Checkout API%".into())]);
        assert_eq!(f("@test.id:abc").sql, "r.test_id::text = $4");
        assert_eq!(f("trigger:manual env:prod team:checkout").sql, "r.trigger = $4 AND t.env = $5 AND t.team = $6");
        assert_eq!(f("service:billing"), ResultFilter { sql: "$4 = ANY(t.tags)".into(), args: vec![Text("service:billing".into())] });
    }

    #[test]
    fn numbers_compare_and_status_codes_take_wildcards() {
        assert_eq!(f("@http.status_code:>=400"), ResultFilter { sql: "r.status_code >= $4".into(), args: vec![Int(400)] });
        assert_eq!(f("@http.status_code:503").sql, "r.status_code = $4");
        assert_eq!(f("@http.status_code:5*"), ResultFilter { sql: "r.status_code::text LIKE $4".into(), args: vec![Text("5%".into())] });
        assert_eq!(f("@result.retries:>0"), ResultFilter { sql: "(r.attempts - 1) > $4".into(), args: vec![Int(0)] });
        assert_eq!(f("@result.duration:<=250.5"), ResultFilter { sql: "r.duration_ms <= $4".into(), args: vec![Float(250.5)] });
        assert!(result_filter("@result.duration:>slow", 1).unwrap_err().contains("number"));
    }

    #[test]
    fn negations_unions_and_free_text() {
        assert_eq!(f("-status:passed"), ResultFilter { sql: "NOT COALESCE((r.passed = $4), false)".into(), args: vec![Bool(true)] });
        assert_eq!(f("@test.type:(http OR ssl)"), ResultFilter { sql: "(t.type = $4 OR t.type = $5)".into(), args: vec![Text("http".into()), Text("ssl".into())] });
        assert_eq!(f("timeout"), ResultFilter { sql: "(t.name ILIKE $4 OR r.error ILIKE $4)".into(), args: vec![Text("%timeout%".into())] });
        assert_eq!(f("100%_off").args, vec![Text("%100\\%\\_off%".into())], "LIKE wildcards in text are literal");
        assert_eq!(f("status:failed AND timeout").args.len(), 2);
        assert!(result_filter("status:maybe", 1).unwrap_err().contains("passed or failed"));
    }

    #[test]
    fn endpoints_come_from_routes_or_span_names() {
        assert_eq!(endpoint_of("GET /orders/{id}", "", ""), Some(("GET".into(), "/orders/{id}".into())));
        assert_eq!(endpoint_of("http.request", "/orders/{id}", "get"), Some(("GET".into(), "/orders/{id}".into())));
        assert_eq!(endpoint_of("flask.request", "/checkout", ""), Some(("".into(), "/checkout".into())));
        assert_eq!(endpoint_of("process-payment", "", "POST"), None);
        assert_eq!(endpoint_of("GET", "", ""), None);
    }

    #[test]
    fn routes_match_paths_with_parameters() {
        assert!(route_matches("/orders/{id}", "/orders/42"));
        assert!(route_matches("/orders/:id/items", "/orders/42/items/"));
        assert!(route_matches("/orders/?", "/orders/42"));
        assert!(route_matches("/static/*", "/static/css/app.css"));
        assert!(route_matches("/", "/"));
        assert!(route_matches("/api/products", "/api/products"));
        assert!(!route_matches("/orders/{id}", "/orders"));
        assert!(!route_matches("/orders/{id}", "/orders/42/items"));
        assert!(!route_matches("/api/products", "/api/orders"));
    }

    #[test]
    fn tests_target_a_method_and_path() {
        assert_eq!(test_target("get", "https://shop.example.com/api/products?page=2"), Some(("GET".into(), "/api/products".into())));
        assert_eq!(test_target("POST", "https://{{ HOST }}/api/orders"), Some(("POST".into(), "/api/orders".into())));
        assert_eq!(test_target("GET", "https://shop.example.com"), Some(("GET".into(), "/".into())));
        assert_eq!(test_target("GET", "not a url"), None);
    }
}
