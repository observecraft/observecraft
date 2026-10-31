//! APM → Settings: how spans flow through APM (overview), the default environment and primary tags,
//! ingestion per service, span pipelines, span-based metrics and retention filters.
//!
//! - Retention filters and span-based metrics are the Datadog API's records
//!   (`datadog_api/apm_config.rs`); these routes read and write the same rows for the UI.
//! - Retention filters are evaluated, not applied: spans reach ClickHouse through the Collector and
//!   are all kept for the org's span retention. A filter's "indexed" spans are those it matches first
//!   (in execution order) times its rate: what Datadog's indexing would keep.
//! - Span-based metrics are computed every minute from `otel.otel_traces` ([`run_spans_metrics`]),
//!   for the minutes two minutes behind now, and written to the metric ingest tables as logs-based
//!   metrics are (`log_metrics.rs`): count → delta sum, distribution → exponential histogram.
//! - Span pipelines are stored and listed; the Collector writes spans directly, so they are not
//!   applied to incoming spans yet.
//! - Queries use Datadog's span search keys on top of [`SpansSchema`] ([`DdSpansSchema`]):
//!   `resource_name`, `operation_name`, `@duration` (ns), `status`, `@_top_level`, `@http.*`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{build_where, parse_query, Param, Schema, SpansSchema, SqlFilter, Term};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::{IntoParams, ToSchema};

use super::datadog_api::apm_config::{self as dd, DdErr, FilterRow, MetricRow};
use super::logs::bind;
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::log_metrics::{Histogram, Point, PointValue, Retention, MISSING};
use crate::state::{AppState, OrgId};

const HTTP_STATUS: &str = "if(SpanAttributes['http.response.status_code'] != '', SpanAttributes['http.response.status_code'], SpanAttributes['http.status_code'])";
const HTTP_METHOD: &str = "if(SpanAttributes['http.request.method'] != '', SpanAttributes['http.request.method'], SpanAttributes['http.method'])";
/// Estimated wire size of a span (bytes): names plus every attribute key and value, plus framing.
const SPAN_BYTES: &str = "(64 + length(SpanName) + length(ServiceName) + arraySum(arrayMap((k, v) -> length(k) + length(v), mapKeys(SpanAttributes), mapValues(SpanAttributes))) \
    + arraySum(arrayMap((k, v) -> length(k) + length(v), mapKeys(ResourceAttributes), mapValues(ResourceAttributes))))";
/// Sampling rate a kept trace root stood for (the Agent's and the tracer's rule rate).
const ROOT_RATE: &str = "(if(toFloat64OrZero(SpanAttributes['_dd.agent_psr']) > 0, toFloat64OrZero(SpanAttributes['_dd.agent_psr']), 1) \
    * if(toFloat64OrZero(SpanAttributes['_dd.rule_psr']) > 0, toFloat64OrZero(SpanAttributes['_dd.rule_psr']), 1))";
const DATADOG_SDK: &str = "ResourceAttributes['telemetry.sdk.name'] = 'datadog'";

/// Datadog's span search keys over `otel.otel_traces`, the rest as [`SpansSchema`].
pub struct DdSpansSchema;

impl Schema for DdSpansSchema {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        match key {
            "resource_name" | "@resource_name" | "operation_name" => ("toString(SpanName)".into(), None),
            "@duration" | "duration" => ("toString(Duration)".into(), None),
            "status" => ("if(StatusCode = 'Error', 'error', 'ok')".into(), None),
            "@_top_level" => ("if(ParentSpanId = '' OR SpanKind IN ('Server', 'Consumer'), '1', '0')".into(), None),
            "@http.status_code" => (HTTP_STATUS.into(), None),
            "@http.method" => (HTTP_METHOD.into(), None),
            k => SpansSchema.field(k),
        }
    }
    fn text_expr(&self) -> &'static str {
        SpansSchema.text_expr()
    }
}

/// The terms of a span query; `*` alone matches everything.
fn terms(q: &str) -> Vec<Term> {
    parse_query(q).into_iter().filter(|t| !matches!(t, Term::Text { text, negate: false } if text == "*")).collect()
}

/// The filter clause of a span query.
pub fn span_filter(q: &str) -> SqlFilter {
    build_where(&DdSpansSchema, &terms(q), None)
}

/// A numeric value of a span at `path` (`@duration` in ns, else an attribute), and its params.
fn value_expr(path: &str) -> (String, Vec<Param>) {
    match path {
        "@duration" | "duration" => ("toNullable(toFloat64(Duration))".into(), vec![]),
        p => {
            let (e, k) = DdSpansSchema.field(p);
            (format!("toFloat64OrNull(toString({e}))"), k.into_iter().collect())
        }
    }
}

fn time_where(from: i64, to: i64) -> (String, Vec<Param>) {
    ("Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".into(), vec![Param::Int(from), Param::Int(to)])
}

#[derive(Deserialize, IntoParams)]
pub struct RangeParams {
    /// Epoch ms.
    pub from: i64,
    /// Epoch ms.
    pub to: i64,
}

impl RangeParams {
    fn check(&self) -> Result<(), ApiError> {
        if self.to < self.from || self.to - self.from > 31 * 86_400_000 {
            return Err(ApiError::BadRequest("range must be at most 31 days".into()));
        }
        Ok(())
    }
}

fn api(e: DdErr) -> ApiError {
    let (status, Json(body)) = e;
    let msg = body["errors"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(": ")).unwrap_or_default();
    match status {
        StatusCode::NOT_FOUND => ApiError::NotFound(msg),
        StatusCode::CONFLICT => ApiError::Conflict(msg),
        StatusCode::FORBIDDEN => ApiError::Forbidden(msg),
        StatusCode::BAD_REQUEST => ApiError::BadRequest(msg),
        _ => ApiError::Internal(anyhow::anyhow!(msg)),
    }
}

#[derive(Serialize, ToSchema, Clone, Copy, Default)]
pub struct ApmPoint {
    /// Bucket start, epoch ms.
    pub t: i64,
    pub v: f64,
}

fn step_ms(from: i64, to: i64) -> i64 {
    let target = ((to - from) / 40).max(60_000);
    [60_000, 300_000, 900_000, 1_800_000, 3_600_000, 3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, 86_400_000].into_iter().find(|s| *s >= target).unwrap_or(86_400_000)
}

// ================================================================ retention filters: what they keep

/// `multiIf(c1, 1, c2, 2, …, 0)`: the first enabled filter matching a span (1-based), and its params.
fn first_match(filters: &[FilterRow]) -> (String, Vec<Param>) {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for (i, f) in filters.iter().enumerate().filter(|(_, f)| f.enabled) {
        let c = span_filter(&f.query);
        parts.push(format!("({}), {}", c.clause, i + 1));
        params.extend(c.params);
    }
    if parts.is_empty() {
        return ("0".into(), vec![]);
    }
    (format!("multiIf({}, 0)", parts.join(", ")), params)
}

/// `multiIf(c1, r1, …, 0)`: the share of a span the first matching filter keeps.
fn kept_share(filters: &[FilterRow]) -> (String, Vec<Param>) {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for f in filters.iter().filter(|f| f.enabled) {
        let c = span_filter(&f.query);
        parts.push(format!("({}), {}", c.clause, f.rate));
        params.extend(c.params);
    }
    if parts.is_empty() {
        return ("0".into(), vec![]);
    }
    (format!("multiIf({}, 0.)", parts.join(", ")), params)
}

/// Spans each filter matches first over the range (index = position in `filters`), and the spans in range.
async fn filter_matches(st: &AppState, org: &str, filters: &[FilterRow], from: i64, to: i64) -> Result<(Vec<u64>, u64), ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct R {
        f: u32,
        n: u64,
    }
    let (m, mut params) = first_match(filters);
    let (tw, tp) = time_where(from, to);
    params.extend(tp);
    let sql = format!("SELECT toUInt32({m}) AS f, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw} GROUP BY f");
    let rows = bind(st.ch(org).query(&sql), &params).fetch_all::<R>().await?;
    let mut out = vec![0; filters.len()];
    let mut total = 0;
    for r in rows {
        total += r.n;
        if r.f > 0 && (r.f as usize) <= out.len() {
            out[r.f as usize - 1] = r.n;
        }
    }
    Ok((out, total))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionFilter {
    pub id: String,
    pub name: String,
    pub query: String,
    /// spans-sampling-processor | spans-errors-sampling-processor | spans-appsec-sampling-processor
    pub filter_type: String,
    pub rate: f64,
    pub trace_rate: f64,
    pub enabled: bool,
    /// One of the org's default filters (can be changed, not deleted).
    pub is_default: bool,
    pub execution_order: i32,
    pub created_by: String,
    /// Epoch seconds (0 for the defaults).
    pub created_at: i64,
    pub modified_by: String,
    pub modified_at: i64,
    /// Spans over the range this filter matches first (in execution order).
    pub spans_matched: u64,
    /// Of those, what its rate keeps.
    pub spans_indexed: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionFilters {
    pub filters: Vec<RetentionFilter>,
    /// Spans ingested over the range.
    pub spans_ingested: u64,
    /// Spans the enabled filters keep over the range.
    pub spans_indexed: u64,
}

fn filter_view(r: &FilterRow, matched: u64) -> RetentionFilter {
    RetentionFilter {
        id: r.id.clone(),
        name: r.name.clone(),
        query: r.query.clone(),
        filter_type: r.filter_type.clone(),
        rate: r.rate,
        trace_rate: r.trace_rate,
        enabled: r.enabled,
        is_default: r.is_default(),
        execution_order: r.execution_order,
        created_by: r.created_by.clone(),
        created_at: r.created_at,
        modified_by: r.modified_by.clone(),
        modified_at: r.modified_at,
        spans_matched: matched,
        spans_indexed: if r.enabled { (matched as f64 * r.rate).round() as u64 } else { 0 },
    }
}

#[utoipa::path(get, path = "/api/v1/apm/settings/retention-filters", operation_id = "list_apm_retention_filters", params(RangeParams), responses((status = 200, body = RetentionFilters)), tag = "apm")]
pub async fn list_filters(State(st): State<AppState>, OrgId(org): OrgId, Query(r): Query<RangeParams>) -> ApiResult<RetentionFilters> {
    r.check()?;
    let rows = dd::list_filter_rows(&st, &org).await?;
    let (matched, total) = filter_matches(&st, &org, &rows, r.from, r.to).await?;
    let filters: Vec<RetentionFilter> = rows.iter().zip(matched).map(|(f, m)| filter_view(f, m)).collect();
    let spans_indexed = filters.iter().map(|f| f.spans_indexed).sum();
    Ok(Json(RetentionFilters { filters, spans_ingested: total, spans_indexed }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionFilterInput {
    pub name: String,
    #[serde(default)]
    pub query: String,
    pub rate: f64,
    #[serde(default)]
    pub trace_rate: f64,
    pub enabled: bool,
}

fn filter_attrs(i: &RetentionFilterInput, kind: &str) -> Map<String, Value> {
    json!({ "name": i.name, "filter": { "query": i.query }, "rate": i.rate, "trace_rate": i.trace_rate, "enabled": i.enabled, "filter_type": kind }).as_object().cloned().unwrap_or_default()
}

#[utoipa::path(post, path = "/api/v1/apm/settings/retention-filters", operation_id = "create_apm_retention_filter", request_body = RetentionFilterInput, responses((status = 200, body = RetentionFilter), (status = 400, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn create_filter(State(st): State<AppState>, Writer(ctx): Writer, Json(i): Json<RetentionFilterInput>) -> ApiResult<RetentionFilter> {
    let r = dd::create_filter_row(&st, &ctx, &filter_attrs(&i, dd::USER_TYPE)).await.map_err(api)?;
    ctx.audit(&st, "apm_retention_filter.created", &r.name, &r.query).await;
    Ok(Json(filter_view(&r, 0)))
}

#[utoipa::path(put, path = "/api/v1/apm/settings/retention-filters/{id}", operation_id = "update_apm_retention_filter", params(("id" = String, Path)), request_body = RetentionFilterInput,
    responses((status = 200, body = RetentionFilter), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn update_filter(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(i): Json<RetentionFilterInput>) -> ApiResult<RetentionFilter> {
    let kind = dd::list_filter_rows(&st, &ctx.org_id).await?.into_iter().find(|f| f.id == id).map(|f| f.filter_type).ok_or_else(|| ApiError::NotFound("retention filter not found".into()))?;
    let r = dd::update_filter_row(&st, &ctx, &id, &filter_attrs(&i, &kind)).await.map_err(api)?;
    ctx.audit(&st, "apm_retention_filter.updated", &r.name, &r.query).await;
    Ok(Json(filter_view(&r, 0)))
}

#[utoipa::path(delete, path = "/api/v1/apm/settings/retention-filters/{id}", operation_id = "delete_apm_retention_filter", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn delete_filter(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    dd::delete_filter_row(&st, &ctx.org_id, &id).await.map_err(api)?;
    ctx.audit(&st, "apm_retention_filter.deleted", &id, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct FilterOrder {
    /// Every retention filter id, in the new execution order.
    pub ids: Vec<String>,
}

#[utoipa::path(put, path = "/api/v1/apm/settings/retention-filters-order", operation_id = "reorder_apm_retention_filters", request_body = FilterOrder, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn reorder_filters(State(st): State<AppState>, Writer(ctx): Writer, Json(o): Json<FilterOrder>) -> Result<StatusCode, ApiError> {
    dd::reorder_rows(&st, &ctx.org_id, &o.ids).await.map_err(api)?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ span-based metrics: definitions

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpanMetricGroupBy {
    pub path: String,
    /// Defaults to the path without `@`.
    #[serde(default)]
    pub tag_name: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpansMetric {
    /// The metric name.
    pub id: String,
    pub query: String,
    /// count | distribution
    pub aggregation_type: String,
    /// Distributions: the numeric path (`@duration`, in ns).
    pub path: Option<String>,
    pub include_percentiles: bool,
    pub group_by: Vec<SpanMetricGroupBy>,
}

fn metric_view(m: &MetricRow) -> SpansMetric {
    SpansMetric {
        id: m.id.clone(),
        query: m.query.clone(),
        aggregation_type: m.aggregation_type.clone(),
        path: m.path.clone(),
        include_percentiles: m.include_percentiles.unwrap_or(false),
        group_by: serde_json::from_value::<Vec<Value>>(m.group_by.clone())
            .unwrap_or_default()
            .iter()
            .map(|g| SpanMetricGroupBy { path: g["path"].as_str().unwrap_or("").into(), tag_name: g["tag_name"].as_str().map(str::to_string) })
            .collect(),
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpansMetricInput {
    /// The metric name (create only).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub query: String,
    /// count | distribution (create only).
    #[serde(default)]
    pub aggregation_type: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub include_percentiles: bool,
    #[serde(default)]
    pub group_by: Vec<SpanMetricGroupBy>,
}

fn metric_attrs(i: &SpansMetricInput, create: bool) -> Map<String, Value> {
    let mut compute = json!({ "include_percentiles": i.include_percentiles });
    if create {
        compute["aggregation_type"] = json!(i.aggregation_type);
        if let Some(p) = i.path.as_deref().filter(|p| !p.trim().is_empty()) {
            compute["path"] = json!(p.trim());
        }
    }
    let group_by: Vec<Value> = i.group_by.iter().map(|g| json!({ "path": g.path, "tag_name": g.tag_name })).collect();
    json!({ "filter": { "query": i.query }, "compute": compute, "group_by": group_by }).as_object().cloned().unwrap_or_default()
}

#[utoipa::path(get, path = "/api/v1/apm/settings/metrics", operation_id = "list_apm_spans_metrics", responses((status = 200, body = Vec<SpansMetric>)), tag = "apm")]
pub async fn list_metrics(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<SpansMetric>> {
    Ok(Json(dd::list_metric_rows(&st, &org).await?.iter().map(metric_view).collect()))
}

#[utoipa::path(post, path = "/api/v1/apm/settings/metrics", operation_id = "create_apm_spans_metric", request_body = SpansMetricInput, responses((status = 200, body = SpansMetric), (status = 400, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn create_metric(State(st): State<AppState>, Writer(ctx): Writer, Json(i): Json<SpansMetricInput>) -> ApiResult<SpansMetric> {
    let m = dd::create_metric_row(&st, &ctx, &i.id, &metric_attrs(&i, true)).await.map_err(api)?;
    ctx.audit(&st, "apm_spans_metric.created", &m.id, &m.query).await;
    Ok(Json(metric_view(&m)))
}

#[utoipa::path(patch, path = "/api/v1/apm/settings/metrics/{id}", operation_id = "update_apm_spans_metric", params(("id" = String, Path)), request_body = SpansMetricInput,
    responses((status = 200, body = SpansMetric), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn update_metric(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(i): Json<SpansMetricInput>) -> ApiResult<SpansMetric> {
    let m = dd::update_metric_row(&st, &ctx.org_id, &id, &metric_attrs(&i, false)).await.map_err(api)?;
    ctx.audit(&st, "apm_spans_metric.updated", &m.id, &m.query).await;
    Ok(Json(metric_view(&m)))
}

#[utoipa::path(delete, path = "/api/v1/apm/settings/metrics/{id}", operation_id = "delete_apm_spans_metric", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn delete_metric(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    dd::delete_metric_row(&st, &ctx.org_id, &id).await.map_err(api)?;
    ctx.audit(&st, "apm_spans_metric.deleted", &id, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ span-based metrics: computation

/// The SQL of one definition over [from, to): rows (minute, group values, histogram bucket, count,
/// sum, min, max); a count has bucket 0 and no values.
fn metric_sql(m: &MetricRow, from: i64, to: i64) -> (String, Vec<Param>) {
    let mut params = Vec::new();
    let mut dims = Vec::new();
    for g in m.group_by.as_array().map(Vec::as_slice).unwrap_or_default() {
        let (e, k) = DdSpansSchema.field(g["path"].as_str().unwrap_or(""));
        params.extend(k);
        dims.push(format!("toString({e})"));
    }
    let g = format!("[{}]", dims.join(", "));
    let distribution = m.aggregation_type == "distribution";
    let value = if distribution {
        let (e, k) = value_expr(m.path.as_deref().unwrap_or("@duration"));
        params.extend(k);
        e
    } else {
        "toNullable(0.)".into()
    };
    let f = span_filter(&m.query);
    params.extend([Param::Int(from), Param::Int(to)]);
    params.extend(f.params);
    let inner = format!(
        "SELECT toInt64(toUnixTimestamp(toStartOfMinute(Timestamp))) * 1000 AS t, {g} AS g, {value} AS v FROM otel.otel_traces \
         WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp < fromUnixTimestamp64Milli(?) AND ({})",
        f.clause
    );
    let sql = if distribution {
        format!(
            "SELECT t, g, toInt32(if(x > 0, ceil(log2(x) * 64) - 1, -2147483648)) AS b, count() AS n, sum(x) AS s, min(x) AS mn, max(x) AS mx \
             FROM (SELECT t, g, assumeNotNull(v) AS x FROM ({inner}) WHERE v IS NOT NULL) GROUP BY t, g, b LIMIT 200000"
        )
    } else {
        format!("SELECT t, g, toInt32(0) AS b, count() AS n, 0. AS s, 0. AS mn, 0. AS mx FROM ({inner}) GROUP BY t, g LIMIT 200000")
    };
    (sql, params)
}

#[derive(clickhouse::Row, Deserialize, Clone, Debug)]
pub struct MetricBucketRow {
    pub t: i64,
    pub g: Vec<String>,
    pub b: i32,
    pub n: u64,
    pub s: f64,
    pub mn: f64,
    pub mx: f64,
}

/// The tag names of a definition's group-by, in order.
fn tag_names(m: &MetricRow) -> Vec<String> {
    m.group_by
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|g| g["tag_name"].as_str().filter(|t| !t.is_empty()).unwrap_or(g["path"].as_str().unwrap_or("").trim_start_matches('@')).to_string())
        .collect()
}

/// Points of one definition from its query rows.
pub fn metric_points(org: &str, m: &MetricRow, rows: &[MetricBucketRow], retention: Retention) -> Vec<Point> {
    let description: Arc<str> = Arc::from(if m.query.trim().is_empty() || m.query.trim() == "*" { "Generated from all spans".to_string() } else { format!("Generated from spans matching: {}", m.query.trim()) });
    let names = tag_names(m);
    let distribution = m.aggregation_type == "distribution";
    let mut series: BTreeMap<(i64, Vec<String>), PointValue> = BTreeMap::new();
    for r in rows {
        let values: Vec<String> = r.g.iter().map(|v| if v.is_empty() { MISSING.to_string() } else { v.chars().take(200).collect() }).collect();
        let entry = series.entry((r.t, values)).or_insert_with(|| if distribution { PointValue::Distribution(Histogram::default()) } else { PointValue::Count(0) });
        match entry {
            PointValue::Count(c) => *c += r.n,
            PointValue::Distribution(h) => {
                (h.min, h.max) = if h.count == 0 { (r.mn, r.mx) } else { (h.min.min(r.mn), h.max.max(r.mx)) };
                h.count += r.n;
                h.sum += r.s;
                if r.b == i32::MIN {
                    h.zero_count += r.n;
                } else {
                    *h.positive.entry(r.b).or_insert(0) += r.n;
                }
            }
        }
    }
    series
        .into_iter()
        .map(|((t, values), value)| Point {
            org: org.to_string(),
            metric: m.id.clone(),
            description: description.clone(),
            retention,
            tags: names.iter().cloned().zip(values).collect(),
            time_ms: t,
            value,
        })
        .collect()
}

async fn retention(st: &AppState, org: &str) -> Retention {
    let r: Option<(i32, i32, i32)> =
        sqlx::query_as("SELECT COALESCE(metrics_retention_days, 0), COALESCE(metrics_5m_days, 0), COALESCE(metrics_1h_days, 0) FROM org_data_settings WHERE org_id = $1")
            .bind(org)
            .fetch_optional(&st.postgres)
            .await
            .unwrap_or(None);
    r.map(|(raw, rollup_5m, rollup_1h)| Retention { raw, rollup_5m, rollup_1h }).unwrap_or_default()
}

/// Minutes behind now the computation stays, so late spans are in.
const LAG_MS: i64 = 2 * 60_000;
/// At most this far back on a first run or after an outage.
const MAX_BACKLOG_MS: i64 = 60 * 60_000;

/// Computes an org's span-based metrics up to two minutes ago; returns the points written.
pub async fn compute_org(st: &AppState, org: &str, now_ms: i64) -> Result<usize, ApiError> {
    let defs = dd::list_metric_rows(st, org).await?;
    let end = (now_ms - LAG_MS) / 60_000 * 60_000;
    let done: Option<i64> = sqlx::query_scalar("SELECT done_until FROM apm_spans_metrics_progress WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    let start = done.unwrap_or(end - 5 * 60_000).max(end - MAX_BACKLOG_MS);
    if defs.is_empty() || start >= end {
        return Ok(0);
    }
    let ret = retention(st, org).await;
    let mut points = Vec::new();
    for m in &defs {
        let (sql, params) = metric_sql(m, start, end);
        match bind(st.ch(org).query(&sql), &params).fetch_all::<MetricBucketRow>().await {
            Ok(rows) => points.extend(metric_points(org, m, &rows, ret)),
            Err(e) => tracing::warn!(error = %e, org, metric = m.id, "span-based metric query failed"),
        }
    }
    let n = points.len();
    let (sums, histograms) = crate::log_metrics::rows_in_scope(&points, "observecraft.spans_metrics");
    for (table, rows) in [("otel.metrics_ingest_sum", sums), ("otel.metrics_ingest_exponential_histogram", histograms)] {
        if !rows.is_empty() {
            crate::logproc::insert_rows(st, table, rows).await.map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
        }
    }
    sqlx::query("INSERT INTO apm_spans_metrics_progress (org_id, done_until) VALUES ($1, $2) ON CONFLICT (org_id) DO UPDATE SET done_until = EXCLUDED.done_until")
        .bind(org)
        .bind(end)
        .execute(&st.postgres)
        .await?;
    Ok(n)
}

/// Every minute, under an advisory lock (one server computes): every org's span-based metrics.
pub async fn run_spans_metrics(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let mut conn = match st.postgres.acquire().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "span-based metrics: postgres unavailable");
                continue;
            }
        };
        if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock(7320)").fetch_one(&mut *conn).await.unwrap_or(false) {
            continue;
        }
        match sqlx::query_scalar::<_, String>("SELECT DISTINCT org_id FROM apm_spans_metrics").fetch_all(&st.postgres).await {
            Ok(orgs) => {
                for org in orgs {
                    if let Err(e) = compute_org(&st, &org, chrono::Utc::now().timestamp_millis()).await {
                        tracing::warn!(error = %e, org, "span-based metrics: computation failed");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "span-based metrics: listing orgs failed"),
        }
        let _ = sqlx::query("SELECT pg_advisory_unlock(7320)").execute(&mut *conn).await;
    }
}

// ================================================================ volumes (Generate Metrics)

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanVolume {
    pub spans_ingested: u64,
    /// Estimated bytes of the spans ingested.
    pub bytes_ingested: u64,
    /// Spans of the last 15 minutes (live search).
    pub spans_live: u64,
    /// Spans the retention filters keep over the range.
    pub spans_indexed: u64,
    /// Days spans are kept for search (the org's span retention).
    pub retention_days: i64,
}

async fn retention_days(st: &AppState, org: &str) -> i64 {
    sqlx::query_scalar::<_, Option<i32>>("SELECT span_retention_days FROM org_data_settings WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await
        .ok()
        .flatten()
        .flatten()
        .filter(|d| *d > 0)
        .map_or(15, i64::from)
}

#[utoipa::path(get, path = "/api/v1/apm/settings/volume", operation_id = "apm_span_volume", params(RangeParams), responses((status = 200, body = SpanVolume)), tag = "apm")]
pub async fn volume(State(st): State<AppState>, OrgId(org): OrgId, Query(r): Query<RangeParams>) -> ApiResult<SpanVolume> {
    r.check()?;
    #[derive(clickhouse::Row, Deserialize)]
    struct V {
        n: u64,
        bytes: u64,
        indexed: f64,
        live: u64,
    }
    let filters = dd::list_filter_rows(&st, &org).await?;
    let (k, mut params) = kept_share(&filters);
    let now = chrono::Utc::now().timestamp_millis();
    params.push(Param::Int(now - 15 * 60_000));
    let (tw, tp) = time_where(r.from, r.to);
    params.extend(tp);
    let sql = format!(
        "SELECT count() AS n, toUInt64(sum({SPAN_BYTES})) AS bytes, sum({k}) AS indexed, countIf(Timestamp >= fromUnixTimestamp64Milli(?)) AS live \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw}"
    );
    let v = bind(st.ch(&org).query(&sql), &params).fetch_one::<V>().await?;
    Ok(Json(SpanVolume { spans_ingested: v.n, bytes_ingested: v.bytes, spans_live: v.live, spans_indexed: v.indexed.round() as u64, retention_days: retention_days(&st, &org).await }))
}

// ================================================================ overview

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApmFlow {
    /// Tracer instances (service × host × language).
    pub sdks: u64,
    /// Hosts sending spans through a Datadog Agent.
    pub agents: u64,
    /// Hosts sending OpenTelemetry spans.
    pub otel_sources: u64,
    pub services: u64,
    /// Spans the tracers created, estimated from the sampling rates of the kept traces.
    pub spans_generated: u64,
    pub spans_ingested: u64,
    /// Spans the retention filters keep.
    pub spans_indexed: u64,
    pub generated: Vec<ApmPoint>,
    pub ingested: Vec<ApmPoint>,
    pub indexed: Vec<ApmPoint>,
    /// Enabled span pipelines.
    pub pipelines: u64,
    /// Service remapping rules (none yet in ObserveCraft).
    pub service_remappings: u64,
    /// Span-based metrics.
    pub metrics: u64,
    /// Enabled retention filters.
    pub retention_filters: u64,
    pub retention_days: i64,
}

#[utoipa::path(get, path = "/api/v1/apm/settings/overview", operation_id = "apm_settings_overview", params(RangeParams), responses((status = 200, body = ApmFlow)), tag = "apm")]
pub async fn overview(State(st): State<AppState>, OrgId(org): OrgId, Query(r): Query<RangeParams>) -> ApiResult<ApmFlow> {
    r.check()?;
    #[derive(clickhouse::Row, Deserialize)]
    struct Totals {
        sdks: u64,
        agents: u64,
        otel: u64,
        services: u64,
        n: u64,
        roots: u64,
        root_weight: f64,
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct Bucket {
        t: i64,
        n: u64,
        indexed: f64,
    }
    let filters = dd::list_filter_rows(&st, &org).await?;
    let (tw, tp) = time_where(r.from, r.to);
    let totals_sql = format!(
        "SELECT uniqExact(ServiceName, ResourceAttributes['host.name'], ResourceAttributes['telemetry.sdk.language']) AS sdks, \
         uniqExactIf(ResourceAttributes['host.name'], {DATADOG_SDK}) AS agents, uniqExactIf(ResourceAttributes['host.name'], NOT ({DATADOG_SDK})) AS otel, \
         uniqExact(ServiceName) AS services, count() AS n, countIf(ParentSpanId = '') AS roots, sumIf(1 / {ROOT_RATE}, ParentSpanId = '') AS root_weight \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw}"
    );
    let step = step_ms(r.from, r.to);
    let (k, mut params) = kept_share(&filters);
    params.extend(tp.clone());
    let series_sql = format!(
        "SELECT toInt64(toUnixTimestamp(toStartOfInterval(Timestamp, INTERVAL {} SECOND))) * 1000 AS t, count() AS n, sum({k}) AS indexed \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw} GROUP BY t ORDER BY t",
        step / 1000
    );
    let (totals, buckets) = tokio::try_join!(bind(st.ch(&org).query(&totals_sql), &tp).fetch_one::<Totals>(), bind(st.ch(&org).query(&series_sql), &params).fetch_all::<Bucket>())?;
    let factor = if totals.roots > 0 { (totals.root_weight / totals.roots as f64).max(1.0) } else { 1.0 };
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM apm_span_pipelines WHERE org_id = $1 AND enabled), (SELECT count(*) FROM apm_spans_metrics WHERE org_id = $1), \
         (SELECT count(*) FROM apm_retention_filters WHERE org_id = $1 AND enabled)",
    )
    .bind(&org)
    .fetch_one(&st.postgres)
    .await?;
    let ingested: Vec<ApmPoint> = buckets.iter().map(|b| ApmPoint { t: b.t, v: b.n as f64 }).collect();
    Ok(Json(ApmFlow {
        sdks: totals.sdks,
        agents: totals.agents,
        otel_sources: totals.otel,
        services: totals.services,
        spans_generated: (totals.n as f64 * factor).round() as u64,
        spans_ingested: totals.n,
        spans_indexed: buckets.iter().map(|b| b.indexed).sum::<f64>().round() as u64,
        generated: buckets.iter().map(|b| ApmPoint { t: b.t, v: (b.n as f64 * factor).round() }).collect(),
        ingested,
        indexed: buckets.iter().map(|b| ApmPoint { t: b.t, v: b.indexed.round() }).collect(),
        pipelines: counts.0 as u64,
        service_remappings: 0,
        metrics: counts.1 as u64,
        retention_filters: counts.2 as u64,
        retention_days: retention_days(&st, &org).await,
    }))
}

// ================================================================ ingestion control

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngestionService {
    pub service: String,
    /// web | db | cache | queue | serverless | custom (from the entry spans' kind and attributes)
    pub kind: String,
    pub env: String,
    /// Tracer language (`telemetry.sdk.language`).
    pub language: String,
    /// Bytes per second of the traces this service starts (it decides their sampling).
    pub downstream_bytes_per_second: f64,
    /// Bytes per second of this service's own spans.
    pub ingested_bytes_per_second: f64,
    /// Ingested share of the downstream traffic (0..1).
    pub traffic_share: f64,
    pub ingested_traces_per_second: f64,
    pub spans_per_second: f64,
    /// Root of at least one trace.
    pub root: bool,
    /// Where sampling is decided: APM AGENT, APM LOCAL, MANUAL, REMOTE, APP & API PROTECTION, RUM, SYNTHETICS, OTEL.
    pub configurations: Vec<String>,
    /// apm | other | otel
    pub managed_by: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngestionOverTime {
    pub apm: Vec<ApmPoint>,
    pub other: Vec<ApmPoint>,
    pub otel: Vec<ApmPoint>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ingestion {
    pub services: Vec<IngestionService>,
    /// Bytes ingested over the range.
    pub bytes_ingested: u64,
    pub spans_ingested: u64,
    /// Bytes per bucket by who decides the sampling.
    pub over_time: IngestionOverTime,
}

/// Sampling decision makers (`_dd.p.dm`, Datadog's decision mechanism) → configuration label.
pub fn configuration(dm: &str) -> Option<&'static str> {
    match dm.trim().trim_start_matches('-') {
        "" => None,
        "0" | "1" | "2" => Some("APM AGENT"),
        "3" => Some("APM LOCAL"),
        "4" => Some("MANUAL"),
        "5" => Some("APP & API PROTECTION"),
        "8" => Some("SINGLE SPAN"),
        "11" | "12" => Some("REMOTE"),
        _ => Some("APM AGENT"),
    }
}

const OTHER_PRODUCTS: [&str; 3] = ["APP & API PROTECTION", "RUM", "SYNTHETICS"];

fn managed_by(configs: &[String]) -> &'static str {
    if configs.iter().any(|c| c == "OTEL") {
        "otel"
    } else if !configs.is_empty() && configs.iter().all(|c| OTHER_PRODUCTS.contains(&c.as_str())) {
        "other"
    } else {
        "apm"
    }
}

#[utoipa::path(get, path = "/api/v1/apm/settings/ingestion", operation_id = "apm_ingestion", params(RangeParams), responses((status = 200, body = Ingestion)), tag = "apm")]
pub async fn ingestion(State(st): State<AppState>, OrgId(org): OrgId, Query(r): Query<RangeParams>) -> ApiResult<Ingestion> {
    r.check()?;
    #[derive(clickhouse::Row, Deserialize)]
    struct S {
        service: String,
        env: String,
        language: String,
        datadog: u8,
        spans: u64,
        bytes: u64,
        traces: u64,
        dms: Vec<String>,
        origins: Vec<String>,
        kinds: Vec<String>,
        db: u8,
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct D {
        service: String,
        bytes: u64,
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct B {
        t: i64,
        apm: u64,
        other: u64,
        otel: u64,
    }
    let (tw, tp) = time_where(r.from, r.to);
    let services_sql = format!(
        "SELECT toString(ServiceName) AS service, anyLast(ResourceAttributes['deployment.environment']) AS env, anyLast(ResourceAttributes['telemetry.sdk.language']) AS language, \
         toUInt8(max({DATADOG_SDK})) AS datadog, count() AS spans, toUInt64(sum({SPAN_BYTES})) AS bytes, uniqExactIf(TraceId, ParentSpanId = '') AS traces, \
         groupUniqArrayIf(10)(SpanAttributes['_dd.p.dm'], SpanAttributes['_dd.p.dm'] != '') AS dms, groupUniqArrayIf(5)(SpanAttributes['_dd.origin'], ParentSpanId = '' AND SpanAttributes['_dd.origin'] != '') AS origins, \
         groupUniqArrayIf(5)(lower(toString(SpanKind)), ParentSpanId = '' OR SpanKind IN ('Server', 'Consumer')) AS kinds, toUInt8(max(SpanAttributes['db.system'] != '' AND SpanKind = 'Server')) AS db \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw} GROUP BY service ORDER BY bytes DESC LIMIT 500"
    );
    let mut dparams = tp.clone();
    dparams.extend(tp.clone());
    let downstream_sql = format!(
        "SELECT r.service AS service, toUInt64(sum(s.b)) AS bytes FROM (SELECT TraceId, {SPAN_BYTES} AS b FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw}) AS s \
         INNER JOIN (SELECT TraceId, any(toString(ServiceName)) AS service FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw} AND ParentSpanId = '' GROUP BY TraceId) AS r \
         ON s.TraceId = r.TraceId GROUP BY service"
    );
    let step = step_ms(r.from, r.to);
    let series_sql = format!(
        "SELECT toInt64(toUnixTimestamp(toStartOfInterval(Timestamp, INTERVAL {} SECOND))) * 1000 AS t, \
         toUInt64(sumIf({SPAN_BYTES}, {DATADOG_SDK} AND SpanAttributes['_dd.p.dm'] NOT IN ('-5') AND SpanAttributes['_dd.origin'] = '')) AS apm, \
         toUInt64(sumIf({SPAN_BYTES}, {DATADOG_SDK} AND (SpanAttributes['_dd.p.dm'] = '-5' OR SpanAttributes['_dd.origin'] != ''))) AS other, \
         toUInt64(sumIf({SPAN_BYTES}, NOT ({DATADOG_SDK}))) AS otel \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {tw} GROUP BY t ORDER BY t",
        step / 1000
    );
    let (rows, down, buckets) = tokio::try_join!(
        bind(st.ch(&org).query(&services_sql), &tp).fetch_all::<S>(),
        bind(st.ch(&org).query(&downstream_sql), &dparams).fetch_all::<D>(),
        bind(st.ch(&org).query(&series_sql), &tp).fetch_all::<B>()
    )?;
    let secs = ((r.to - r.from) as f64 / 1000.0).max(1.0);
    let down: HashMap<String, u64> = down.into_iter().map(|d| (d.service, d.bytes)).collect();
    let services = rows
        .into_iter()
        .map(|s| {
            let mut configs: BTreeSet<String> = BTreeSet::new();
            if s.datadog == 0 {
                configs.insert("OTEL".into());
            }
            for dm in &s.dms {
                if let Some(c) = configuration(dm) {
                    configs.insert(c.into());
                }
            }
            for o in &s.origins {
                configs.insert(if o.starts_with("synthetics") { "SYNTHETICS".into() } else if o == "rum" { "RUM".into() } else { o.to_uppercase() });
            }
            if s.datadog == 1 && configs.is_empty() {
                configs.insert("APM AGENT".into());
            }
            let configs: Vec<String> = configs.into_iter().collect();
            let downstream = down.get(&s.service).copied().unwrap_or(0);
            let kind = if s.db == 1 {
                "db"
            } else if s.kinds.iter().any(|k| k == "consumer") {
                "queue"
            } else if s.kinds.iter().any(|k| k == "server") {
                "web"
            } else {
                "custom"
            };
            IngestionService {
                managed_by: managed_by(&configs).into(),
                kind: kind.into(),
                env: s.env,
                language: s.language,
                downstream_bytes_per_second: downstream as f64 / secs,
                ingested_bytes_per_second: s.bytes as f64 / secs,
                traffic_share: if downstream > 0 { (s.bytes as f64 / downstream as f64).min(1.0) } else { 1.0 },
                ingested_traces_per_second: s.traces as f64 / secs,
                spans_per_second: s.spans as f64 / secs,
                root: downstream > 0,
                configurations: configs,
                service: s.service,
            }
        })
        .collect::<Vec<_>>();
    let pt = |f: fn(&B) -> u64| buckets.iter().map(|b| ApmPoint { t: b.t, v: f(b) as f64 }).collect::<Vec<_>>();
    Ok(Json(Ingestion {
        bytes_ingested: buckets.iter().map(|b| b.apm + b.other + b.otel).sum(),
        spans_ingested: services.iter().map(|s| (s.spans_per_second * secs).round() as u64).sum(),
        over_time: IngestionOverTime { apm: pt(|b| b.apm), other: pt(|b| b.other), otel: pt(|b| b.otel) },
        services,
    }))
}

// ================================================================ default settings

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApmEnvOption {
    pub env: String,
    /// Services seen in the past day.
    pub services: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApmDefaults {
    /// The org's choice (null: automatically detected).
    pub default_env: Option<String>,
    /// The environment with the most active services.
    pub detected_env: Option<String>,
    pub envs: Vec<ApmEnvOption>,
    /// Tag names added to APM metrics as dimensions.
    pub primary_tags: Vec<String>,
    pub default_primary_tag: Option<String>,
    /// Host and container tags seen on spans (candidates for primary tags).
    pub tag_candidates: Vec<String>,
}

async fn defaults_of(st: &AppState, org: &str) -> Result<ApmDefaults, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct E {
        env: String,
        services: u64,
    }
    let now = chrono::Utc::now().timestamp_millis();
    let envs = st
        .ch(org)
        .query(
            "SELECT ResourceAttributes['deployment.environment'] AS env, uniqExact(ServiceName) AS services FROM otel.otel_traces \
             WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND env != '' GROUP BY env ORDER BY services DESC, env LIMIT 50",
        )
        .bind(now - 86_400_000)
        .fetch_all::<E>()
        .await?;
    let keys = st
        .ch(org)
        .query(
            "SELECT k FROM (SELECT arrayJoin(mapKeys(ResourceAttributes)) AS k FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) LIMIT 100000) \
             WHERE k NOT LIKE 'oc.%' AND k NOT LIKE 'telemetry.%' GROUP BY k ORDER BY k LIMIT 100",
        )
        .bind(now - 3_600_000)
        .fetch_all::<String>()
        .await?;
    let row: Option<(Option<String>, Value, Option<String>)> =
        sqlx::query_as("SELECT default_env, primary_tags, default_primary_tag FROM apm_org_settings WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    let (default_env, tags, default_tag) = row.unwrap_or((None, json!([]), None));
    Ok(ApmDefaults {
        detected_env: envs.first().map(|e| e.env.clone()),
        envs: envs.into_iter().map(|e| ApmEnvOption { env: e.env, services: e.services }).collect(),
        default_env,
        primary_tags: serde_json::from_value(tags).unwrap_or_default(),
        default_primary_tag: default_tag,
        tag_candidates: keys,
    })
}

#[utoipa::path(get, path = "/api/v1/apm/settings/defaults", operation_id = "get_apm_defaults", responses((status = 200, body = ApmDefaults)), tag = "apm")]
pub async fn get_defaults(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<ApmDefaults> {
    Ok(Json(defaults_of(&st, &org).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApmDefaultsInput {
    /// null: automatically detected.
    pub default_env: Option<String>,
    #[serde(default)]
    pub primary_tags: Vec<String>,
    pub default_primary_tag: Option<String>,
}

/// A tag name: lowercase letters, digits, `_ - . /`, at most 100 characters.
fn tag_name_ok(t: &str) -> bool {
    !t.is_empty() && t.len() <= 100 && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
}

#[utoipa::path(put, path = "/api/v1/apm/settings/defaults", operation_id = "update_apm_defaults", request_body = ApmDefaultsInput, responses((status = 200, body = ApmDefaults), (status = 400, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn update_defaults(State(st): State<AppState>, Writer(ctx): Writer, Json(i): Json<ApmDefaultsInput>) -> ApiResult<ApmDefaults> {
    let mut tags: Vec<String> = Vec::new();
    for t in i.primary_tags.iter().map(|t| t.trim().to_lowercase()) {
        if !tag_name_ok(&t) {
            return Err(ApiError::BadRequest(format!("'{t}' is not a valid tag name")));
        }
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    if tags.len() > 3 {
        return Err(ApiError::BadRequest("at most 3 primary tags".into()));
    }
    let default_tag = i.default_primary_tag.map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty());
    if default_tag.as_ref().is_some_and(|t| !tags.contains(t)) {
        return Err(ApiError::BadRequest("the default primary tag must be one of the primary tags".into()));
    }
    let env = i.default_env.map(|e| e.trim().trim_start_matches("env:").to_string()).filter(|e| !e.is_empty());
    sqlx::query(
        "INSERT INTO apm_org_settings (org_id, default_env, primary_tags, default_primary_tag) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (org_id) DO UPDATE SET default_env = EXCLUDED.default_env, primary_tags = EXCLUDED.primary_tags, default_primary_tag = EXCLUDED.default_primary_tag, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&env)
    .bind(json!(tags))
    .bind(&default_tag)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "apm_defaults.updated", env.as_deref().unwrap_or("auto"), &tags.join(", ")).await;
    Ok(Json(defaults_of(&st, &ctx.org_id).await?))
}

// ================================================================ span pipelines

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpanProcessor {
    /// remapper (copy an attribute to another name) | category (set an attribute from queries)
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    /// remapper: source attribute paths.
    #[serde(default)]
    pub sources: Vec<String>,
    /// remapper / category: the attribute written.
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub preserve_source: bool,
    /// category: (name, query) pairs; the first matching query's name is written.
    #[serde(default)]
    pub categories: Vec<SpanCategory>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct SpanCategory {
    pub name: String,
    pub query: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanPipeline {
    pub id: String,
    pub name: String,
    pub query: String,
    pub enabled: bool,
    pub processors: Vec<SpanProcessor>,
    pub position: i32,
    pub updated_by: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanPipelineInput {
    pub name: String,
    #[serde(default)]
    pub query: String,
    pub enabled: bool,
    #[serde(default)]
    pub processors: Vec<SpanProcessor>,
}

#[derive(sqlx::FromRow)]
struct PipelineRow {
    id: String,
    name: String,
    query: String,
    enabled: bool,
    processors: Value,
    position: i32,
    updated_by_name: String,
    updated_at: i64,
}

fn pipeline_view(p: PipelineRow) -> SpanPipeline {
    SpanPipeline {
        id: p.id,
        name: p.name,
        query: p.query,
        enabled: p.enabled,
        processors: serde_json::from_value(p.processors).unwrap_or_default(),
        position: p.position,
        updated_by: p.updated_by_name,
        updated_at: p.updated_at,
    }
}

const PIPELINE_COLUMNS: &str = "id::text AS id, name, query, enabled, processors, position, updated_by_name, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

fn check_pipeline(i: &SpanPipelineInput) -> Result<(), ApiError> {
    if i.name.trim().is_empty() || i.name.len() > 200 {
        return Err(ApiError::BadRequest("a pipeline needs a name (at most 200 characters)".into()));
    }
    for p in &i.processors {
        match p.kind.as_str() {
            "remapper" if !p.sources.is_empty() && !p.target.trim().is_empty() => {}
            "category" if !p.categories.is_empty() && !p.target.trim().is_empty() => {}
            "remapper" | "category" => return Err(ApiError::BadRequest(format!("processor '{}' needs sources or categories and a target", p.name))),
            k => return Err(ApiError::BadRequest(format!("unknown processor type '{k}'"))),
        }
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/apm/settings/pipelines", operation_id = "list_apm_span_pipelines", responses((status = 200, body = Vec<SpanPipeline>)), tag = "apm")]
pub async fn list_pipelines(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<SpanPipeline>> {
    let rows: Vec<PipelineRow> = sqlx::query_as(&format!("SELECT {PIPELINE_COLUMNS} FROM apm_span_pipelines WHERE org_id = $1 ORDER BY position, updated_at")).bind(&org).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(pipeline_view).collect()))
}

async fn pipeline(st: &AppState, org: &str, id: &str) -> Result<SpanPipeline, ApiError> {
    let row: Option<PipelineRow> = sqlx::query_as(&format!("SELECT {PIPELINE_COLUMNS} FROM apm_span_pipelines WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(pipeline_view).ok_or_else(|| ApiError::NotFound("pipeline not found".into()))
}

#[utoipa::path(post, path = "/api/v1/apm/settings/pipelines", operation_id = "create_apm_span_pipeline", request_body = SpanPipelineInput, responses((status = 200, body = SpanPipeline), (status = 400, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn create_pipeline(State(st): State<AppState>, Writer(ctx): Writer, Json(i): Json<SpanPipelineInput>) -> ApiResult<SpanPipeline> {
    check_pipeline(&i)?;
    let id: String = sqlx::query_scalar(
        "INSERT INTO apm_span_pipelines (org_id, name, query, enabled, processors, position, updated_by_name) \
         VALUES ($1, $2, $3, $4, $5, (SELECT COALESCE(max(position), -1) + 1 FROM apm_span_pipelines WHERE org_id = $1), $6) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(i.name.trim())
    .bind(i.query.trim())
    .bind(i.enabled)
    .bind(json!(i.processors))
    .bind(&ctx.name)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "apm_span_pipeline.created", i.name.trim(), i.query.trim()).await;
    Ok(Json(pipeline(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/apm/settings/pipelines/{id}", operation_id = "update_apm_span_pipeline", params(("id" = String, Path)), request_body = SpanPipelineInput,
    responses((status = 200, body = SpanPipeline), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn update_pipeline(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(i): Json<SpanPipelineInput>) -> ApiResult<SpanPipeline> {
    check_pipeline(&i)?;
    let n = sqlx::query("UPDATE apm_span_pipelines SET name = $3, query = $4, enabled = $5, processors = $6, updated_by_name = $7, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(i.name.trim())
        .bind(i.query.trim())
        .bind(i.enabled)
        .bind(json!(i.processors))
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("pipeline not found".into()));
    }
    ctx.audit(&st, "apm_span_pipeline.updated", i.name.trim(), i.query.trim()).await;
    Ok(Json(pipeline(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/apm/settings/pipelines/{id}", operation_id = "delete_apm_span_pipeline", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "apm")]
pub async fn delete_pipeline(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("DELETE FROM apm_span_pipelines WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("pipeline not found".into()));
    }
    ctx.audit(&st, "apm_span_pipeline.deleted", &id, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(kind: &str, path: Option<&str>, group_by: Value) -> MetricRow {
        MetricRow { id: "checkout.spans".into(), query: "service:storefront-api".into(), aggregation_type: kind.into(), path: path.map(str::to_string), include_percentiles: None, group_by }
    }

    #[test]
    fn datadog_span_keys_map_onto_the_span_columns() {
        let f = span_filter("service:storefront-api resource_name:\"GET /cart\" status:error @http.status_code:5* -env:qa *");
        assert_eq!(
            f.clause,
            "(ServiceName = ?) AND (toString(SpanName) = ?) AND (if(StatusCode = 'Error', 'error', 'ok') = ?) AND (".to_string()
                + HTTP_STATUS
                + " ILIKE ?) AND NOT (ResourceAttributes[?] = ?)"
        );
        assert_eq!(span_filter("*").clause, "1");
    }

    #[test]
    fn first_matching_enabled_filter_wins() {
        let row = |id: &str, q: &str, enabled: bool, rate: f64| FilterRow {
            id: id.into(),
            name: id.into(),
            query: q.into(),
            filter_type: dd::USER_TYPE.into(),
            rate,
            trace_rate: 0.0,
            enabled,
            execution_order: 0,
            created_by: String::new(),
            created_at: 0,
            modified_by: String::new(),
            modified_at: 0,
        };
        let filters = [row("a", "status:error", true, 1.0), row("b", "service:billing", false, 1.0), row("c", "*", true, 0.1)];
        let (m, p) = first_match(&filters);
        assert_eq!(m, "multiIf(((if(StatusCode = 'Error', 'error', 'ok') = ?)), 1, (1), 3, 0)");
        assert_eq!(p, vec![Param::Str("error".into())]);
        assert_eq!(kept_share(&filters).0, "multiIf(((if(StatusCode = 'Error', 'error', 'ok') = ?)), 1, (1), 0.1, 0.)");
        assert_eq!(filter_view(&filters[2], 1000).spans_indexed, 100);
        assert_eq!(filter_view(&filters[1], 1000).spans_indexed, 0);
    }

    #[test]
    fn counts_become_points_per_minute_and_group() {
        let m = metric("count", None, json!([{"path": "resource_name", "tag_name": "resource_name"}, {"path": "env"}]));
        let row = |t: i64, g: [&str; 2], n: u64| MetricBucketRow { t, g: g.iter().map(|s| s.to_string()).collect(), b: 0, n, s: 0.0, mn: 0.0, mx: 0.0 };
        let points = metric_points("o", &m, &[row(60_000, ["GET /cart", "prod"], 3), row(60_000, ["GET /cart", ""], 2), row(120_000, ["GET /cart", "prod"], 1)], Retention::default());
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].tags, vec![("resource_name".to_string(), "GET /cart".to_string()), ("env".to_string(), MISSING.to_string())]);
        assert_eq!(points[0].value, PointValue::Count(2));
        assert_eq!(&*points[0].description, "Generated from spans matching: service:storefront-api");
    }

    #[test]
    fn distributions_fold_their_buckets() {
        let m = metric("distribution", Some("@duration"), json!([]));
        let row = |b: i32, n: u64, s: f64, mn: f64, mx: f64| MetricBucketRow { t: 0, g: vec![], b, n, s, mn, mx };
        let b = crate::log_metrics::bucket_index(2_000_000.0);
        let points = metric_points("o", &m, &[row(b, 2, 4_000_000.0, 1_990_000.0, 2_010_000.0), row(i32::MIN, 1, 0.0, 0.0, 0.0)], Retention::default());
        let PointValue::Distribution(h) = &points[0].value else { panic!() };
        assert_eq!((h.count, h.zero_count, h.min, h.max), (3, 1, 0.0, 2_010_000.0));
        assert_eq!(h.positive.get(&b), Some(&2));
        let (sql, params) = metric_sql(&m, 0, 60_000);
        assert!(sql.contains("FROM otel.otel_traces WHERE OrgId = {oc_org:String}") && sql.contains("toNullable(toFloat64(Duration))"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn sampling_mechanisms_name_the_configuration() {
        assert_eq!(configuration("-0"), Some("APM AGENT"));
        assert_eq!(configuration("-3"), Some("APM LOCAL"));
        assert_eq!(configuration("-5"), Some("APP & API PROTECTION"));
        assert_eq!(configuration(""), None);
        assert_eq!(managed_by(&["OTEL".into()]), "otel");
        assert_eq!(managed_by(&["RUM".into()]), "other");
        assert_eq!(managed_by(&["APM AGENT".into(), "RUM".into()]), "apm");
    }

    #[test]
    fn primary_tags_are_tag_names() {
        assert!(tag_name_ok("availability-zone") && tag_name_ok("datacenter"));
        assert!(!tag_name_ok("Data Center") && !tag_name_ok(""));
    }

    #[tokio::test]
    async fn span_metrics_and_retention_counts_live() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "apmset").await;
        let now = chrono::Utc::now().timestamp_millis();
        let minute = (now - LAG_MS) / 60_000 * 60_000 - 60_000;
        st.ch(&org)
            .query(&format!(
                "INSERT INTO otel.otel_traces (Timestamp, TraceId, SpanId, SpanName, SpanKind, ServiceName, ResourceAttributes, SpanAttributes, Duration, StatusCode) \
                 SELECT fromUnixTimestamp64Milli(toInt64({minute} + number * 1000)), lower(hex(number + 1)), lower(hex(number + 1)), 'GET /cart', 'Server', 'storefront-api', \
                        map('deployment.environment', 'prod', 'oc.org_id', '{org}', 'telemetry.sdk.name', 'datadog'), map('_dd.p.dm', '-0'), 2000000 * (number + 1), \
                        if(number = 0, 'Error', 'Ok') FROM numbers(4)"
            ))
            .execute()
            .await
            .expect("insert spans");
        // Retention filters: the default error filter matches the error span first.
        let filters = dd::list_filter_rows(st, &org).await.unwrap();
        let (matched, total) = filter_matches(st, &org, &filters, minute, minute + 60_000).await.unwrap();
        assert_eq!(total, 4);
        assert_eq!(matched[filters.iter().position(|f| f.id == "errors-default").unwrap()], 1);
        // A span-based distribution of the minute.
        let m = metric("distribution", Some("@duration"), json!([{"path": "resource_name"}]));
        let (sql, params) = metric_sql(&m, minute, minute + 60_000);
        let got = bind(st.ch(&org).query(&sql), &params).fetch_all::<MetricBucketRow>().await.unwrap();
        let points = metric_points(&org, &m, &got, Retention::default());
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].tags, vec![("resource_name".to_string(), "GET /cart".to_string())]);
        let PointValue::Distribution(h) = &points[0].value else { panic!() };
        assert_eq!((h.count, h.min, h.max), (4, 2_000_000.0, 8_000_000.0));
    }
}
