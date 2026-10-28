//! LLM Observability over the spans already stored in `otel.otel_traces`: overview (requests,
//! errors, latency, tokens, estimated cost), the list of traces that contain LLM spans, facets,
//! one trace with its LLM details, and the built-in evaluations. `llm_attrs` says which spans are
//! LLM spans and how their attributes read; `llm_checks` holds the evaluations.

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{build_where, parse_query, Param, SpansSchema};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::apm::TraceDetail;
use super::llm_attrs::{self as attrs, LlmMessage, LlmSpanInfo};
use super::llm_checks::{self as checks, CallSample, CheckOutcome};
use super::logs::{bind, TimeRange};
use super::spans::SpanFacetValue;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// Facet dimensions: (API name, SQL expression).
fn dimensions() -> Vec<(&'static str, String)> {
    vec![
        ("ml_app", attrs::ml_app_sql()),
        ("service", "toString(ServiceName)".into()),
        ("model", attrs::first_sql(attrs::MODEL_KEYS)),
        ("provider", attrs::first_sql(attrs::PROVIDER_KEYS)),
        ("kind", attrs::kind_sql()),
        ("operation", attrs::first_sql(attrs::OPERATION_KEYS)),
        ("env", "ResourceAttributes['deployment.environment']".into()),
        ("status", "if(StatusCode = 'Error', 'error', 'ok')".into()),
    ]
}

fn dimension(name: &str) -> Result<String, ApiError> {
    dimensions().into_iter().find(|d| d.0 == name).map(|d| d.1).ok_or_else(|| ApiError::BadRequest(format!("unknown dimension '{name}'")))
}

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LlmQuery {
    pub range: TimeRange,
    /// Span search syntax (`service:support-assistant @gen_ai.request.model:gpt-4o*`).
    #[serde(default)]
    pub query: String,
    /// Facet selections: dimension (ml_app, service, model, provider, kind, operation, env, status) → values.
    #[serde(default)]
    pub filters: BTreeMap<String, Vec<String>>,
}

impl LlmQuery {
    fn narrowed(&self) -> bool {
        !self.query.trim().is_empty() || self.filters.values().any(|v| !v.is_empty())
    }
}

/// The range and the LLM-span predicate.
fn base_clause(range: &TimeRange) -> Result<(String, Vec<Param>), ApiError> {
    range.validate()?;
    Ok((format!("Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND {}", attrs::is_llm_sql()), vec![Param::Int(range.from), Param::Int(range.to)]))
}

/// Base clause plus search and facet selections; `skip` leaves out one facet's own selection.
fn where_clause(q: &LlmQuery, skip: Option<&str>) -> Result<(String, Vec<Param>), ApiError> {
    let (base, mut params) = base_clause(&q.range)?;
    let mut parts = vec![base];
    if !q.query.trim().is_empty() {
        let f = build_where(&SpansSchema, &parse_query(&q.query), None);
        parts.push(format!("({})", f.clause));
        params.extend(f.params);
    }
    for (dim, values) in &q.filters {
        if values.is_empty() || Some(dim.as_str()) == skip {
            continue;
        }
        parts.push(format!("{} IN ({})", dimension(dim)?, vec!["?"; values.len()].join(", ")));
        params.extend(values.iter().map(|v| Param::Str(v.clone())));
    }
    Ok((parts.join(" AND "), params))
}

fn step_for(range: &TimeRange, buckets: i64) -> i64 {
    let raw = (range.to - range.from) / buckets.max(1);
    const STEPS: [i64; 12] = [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, 86_400_000];
    STEPS.iter().copied().find(|s| *s >= raw).unwrap_or(86_400_000)
}

fn bucket_times(range: &TimeRange, step: i64) -> Vec<i64> {
    let first = range.from / step * step;
    (0..).map(|i| first + i * step).take_while(|t| *t <= range.to).collect()
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    if v.is_finite() {
        (v * f).round() / f
    } else {
        0.0
    }
}

// ================================================================ overview

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmOverviewRequest {
    #[serde(flatten)]
    pub q: LlmQuery,
    /// Series grouping: model (default) or ml_app.
    pub group_by: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelUsage {
    pub model: String,
    pub provider: String,
    pub requests: u64,
    pub errors: u64,
    pub input_tokens: f64,
    pub output_tokens: f64,
    /// Estimated USD from the built-in list prices; null when the model has no price.
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmAppUsage {
    pub ml_app: String,
    pub requests: u64,
    pub errors: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub input_tokens: f64,
    pub output_tokens: f64,
    /// Estimated USD over the app's priced models.
    pub estimated_cost_usd: f64,
    pub models: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct LlmSeries {
    pub group: String,
    pub values: Vec<Option<f64>>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmOverview {
    /// Model calls (kind llm).
    pub requests: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// Tokens of model and embedding calls.
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub total_tokens: f64,
    /// Sum over models with a list price; see `unpricedModels`.
    pub estimated_cost_usd: f64,
    pub unpriced_models: Vec<String>,
    pub models: Vec<LlmModelUsage>,
    pub apps: Vec<LlmAppUsage>,
    pub step_ms: i64,
    pub times: Vec<i64>,
    pub request_series: Vec<LlmSeries>,
    pub token_series: Vec<LlmSeries>,
    /// p95 latency of model calls per bucket.
    pub latency_series: Vec<LlmSeries>,
    /// Estimated USD per bucket.
    pub cost_series: Vec<LlmSeries>,
}

/// Usage of one (app, model) pair.
struct Usage {
    app: String,
    model: String,
    provider: String,
    requests: u64,
    errors: u64,
    tin: f64,
    tout: f64,
}

/// Totals, per-model and per-app usage from (app, model) rows; latency per app comes separately.
fn summarize(rows: Vec<Usage>, app_latency: &HashMap<String, (f64, f64)>) -> (Vec<LlmModelUsage>, Vec<LlmAppUsage>, Vec<String>) {
    let mut models: Vec<LlmModelUsage> = Vec::new();
    let mut apps: Vec<LlmAppUsage> = Vec::new();
    for u in rows {
        let cost = attrs::estimate_cost(&u.model, u.tin, u.tout);
        match models.iter_mut().find(|m| m.model == u.model) {
            Some(m) => {
                m.requests += u.requests;
                m.errors += u.errors;
                m.input_tokens += u.tin;
                m.output_tokens += u.tout;
                m.estimated_cost_usd = m.estimated_cost_usd.zip(cost).map(|(a, b)| a + b);
            }
            None => models.push(LlmModelUsage {
                model: u.model.clone(),
                provider: u.provider.clone(),
                requests: u.requests,
                errors: u.errors,
                input_tokens: u.tin,
                output_tokens: u.tout,
                estimated_cost_usd: cost,
            }),
        }
        let app = match apps.iter_mut().position(|a| a.ml_app == u.app) {
            Some(i) => &mut apps[i],
            None => {
                let (p50, p95) = app_latency.get(&u.app).copied().unwrap_or((0.0, 0.0));
                apps.push(LlmAppUsage { ml_app: u.app.clone(), requests: 0, errors: 0, p50_ms: p50, p95_ms: p95, input_tokens: 0.0, output_tokens: 0.0, estimated_cost_usd: 0.0, models: vec![] });
                apps.last_mut().unwrap()
            }
        };
        app.requests += u.requests;
        app.errors += u.errors;
        app.input_tokens += u.tin;
        app.output_tokens += u.tout;
        app.estimated_cost_usd += cost.unwrap_or(0.0);
        if !u.model.is_empty() && !app.models.contains(&u.model) {
            app.models.push(u.model);
        }
    }
    let unpriced = models
        .iter()
        .filter(|m| m.estimated_cost_usd.is_none() && m.input_tokens + m.output_tokens > 0.0)
        .map(|m| if m.model.is_empty() { "(no model)".to_string() } else { m.model.clone() })
        .collect();
    for m in &mut models {
        m.estimated_cost_usd = m.estimated_cost_usd.map(|c| round(c, 6));
    }
    for a in &mut apps {
        a.estimated_cost_usd = round(a.estimated_cost_usd, 6);
    }
    models.sort_by(|a, b| b.requests.cmp(&a.requests).then(a.model.cmp(&b.model)));
    apps.sort_by(|a, b| b.requests.cmp(&a.requests).then(a.ml_app.cmp(&b.ml_app)));
    (models, apps, unpriced)
}

#[utoipa::path(post, path = "/api/v1/llm/overview", operation_id = "llm_overview", request_body = LlmOverviewRequest, responses((status = 200, body = LlmOverview)), tag = "llm")]
pub async fn overview(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LlmOverviewRequest>) -> ApiResult<LlmOverview> {
    let (w, params) = where_clause(&req.q, None)?;
    let kind = attrs::kind_sql();
    let call = attrs::is_call_sql();
    let (model, provider, app) = (attrs::first_sql(attrs::MODEL_KEYS), attrs::first_sql(attrs::PROVIDER_KEYS), attrs::ml_app_sql());
    let (tin, tout) = (attrs::input_tokens_sql(), attrs::output_tokens_sql());
    #[derive(clickhouse::Row, Deserialize)]
    struct M {
        app: String,
        model: String,
        provider: String,
        requests: u64,
        errors: u64,
        tin: f64,
        tout: f64,
    }
    let usage_q = bind(
        st.ch(&org).query(&format!(
            "SELECT {app} AS app, {model} AS model, any({provider}) AS provider, countIf({kind} = 'llm') AS requests, countIf({kind} = 'llm' AND StatusCode = 'Error') AS errors, \
             sum({tin}) AS tin, sum({tout}) AS tout FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND {call} GROUP BY app, model ORDER BY requests DESC LIMIT 1000"
        )),
        &params,
    );
    #[derive(clickhouse::Row, Deserialize)]
    struct L {
        app: String,
        p50: f64,
        p95: f64,
    }
    let lat_sql = |by: &str| {
        format!(
            "SELECT {by} AS app, quantileIf(0.5)(Duration, {kind} = 'llm') / 1e6 AS p50, quantileIf(0.95)(Duration, {kind} = 'llm') / 1e6 AS p95 \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND {call} GROUP BY app LIMIT 1000"
        )
    };
    let total_lat_q = bind(st.ch(&org).query(&lat_sql("''")), &params);
    let app_lat_q = bind(st.ch(&org).query(&lat_sql(&app)), &params);
    let group = if req.group_by.as_deref() == Some("ml_app") { app.clone() } else { model.clone() };
    let step = step_for(&req.q.range, 60);
    #[derive(clickhouse::Row, Deserialize)]
    struct B {
        g: String,
        model: String,
        t: i64,
        requests: u64,
        tin: f64,
        tout: f64,
        p95: f64,
    }
    let series_q = bind(
        st.ch(&org).query(&format!(
            "SELECT {group} AS g, {model} AS model, intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, countIf({kind} = 'llm') AS requests, sum({tin}) AS tin, \
             sum({tout}) AS tout, quantileIf(0.95)(Duration, {kind} = 'llm') / 1e6 AS p95 FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND {call} GROUP BY g, model, t LIMIT 50000"
        )),
        &params,
    );
    let (usage, total_lat, app_lat, buckets) = tokio::try_join!(usage_q.fetch_all::<M>(), total_lat_q.fetch_all::<L>(), app_lat_q.fetch_all::<L>(), series_q.fetch_all::<B>())?;
    let total_lat = total_lat.first().map(|l| (l.p50, l.p95)).unwrap_or((0.0, 0.0));
    let app_latency: HashMap<String, (f64, f64)> = app_lat.into_iter().map(|l| (l.app, (round(l.p50, 2), round(l.p95, 2)))).collect();
    let rows = usage.into_iter().map(|m| Usage { app: m.app, model: m.model, provider: m.provider, requests: m.requests, errors: m.errors, tin: m.tin, tout: m.tout }).collect();
    let (models, apps, unpriced_models) = summarize(rows, &app_latency);
    let requests: u64 = models.iter().map(|m| m.requests).sum();
    let errors: u64 = models.iter().map(|m| m.errors).sum();
    let (input_tokens, output_tokens) = (models.iter().map(|m| m.input_tokens).sum::<f64>(), models.iter().map(|m| m.output_tokens).sum::<f64>());
    let times = bucket_times(&req.q.range, step);
    let points = buckets
        .into_iter()
        .map(|b| SeriesPoint { cost: attrs::estimate_cost(&b.model, b.tin, b.tout).unwrap_or(0.0), group: b.g, t: b.t, requests: b.requests as f64, tokens: b.tin + b.tout, p95: b.p95 })
        .collect();
    let [request_series, token_series, latency_series, cost_series] = series(&times, step, points);
    Ok(Json(LlmOverview {
        requests,
        errors,
        error_rate: if requests == 0 { 0.0 } else { round(errors as f64 / requests as f64, 4) },
        p50_ms: round(total_lat.0, 2),
        p95_ms: round(total_lat.1, 2),
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
        estimated_cost_usd: round(models.iter().filter_map(|m| m.estimated_cost_usd).sum(), 6),
        unpriced_models,
        models,
        apps,
        step_ms: step,
        times,
        request_series,
        token_series,
        latency_series,
        cost_series,
    }))
}

/// One bucket of one group (and model, when grouping by app).
struct SeriesPoint {
    group: String,
    t: i64,
    requests: f64,
    tokens: f64,
    p95: f64,
    cost: f64,
}

/// Buckets → the top 8 groups by requests (then tokens) as request, token, p95 latency and cost
/// series. Counts are 0 in empty buckets, latency null; a group's p95 over several models is the
/// highest of them.
fn series(times: &[i64], step: i64, points: Vec<SeriesPoint>) -> [Vec<LlmSeries>; 4] {
    let first = times.first().copied().unwrap_or(0);
    let mut totals: HashMap<String, (f64, f64)> = HashMap::new();
    for p in &points {
        let t = totals.entry(p.group.clone()).or_default();
        t.0 += p.requests;
        t.1 += p.tokens;
    }
    let mut groups: Vec<(String, (f64, f64))> = totals.into_iter().collect();
    groups.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0).then(b.1 .1.total_cmp(&a.1 .1)).then(a.0.cmp(&b.0)));
    groups.truncate(8);
    let index: HashMap<&str, usize> = groups.iter().enumerate().map(|(i, g)| (g.0.as_str(), i)).collect();
    let n = times.len();
    let mut out: [Vec<Vec<Option<f64>>>; 4] =
        [vec![vec![Some(0.0); n]; groups.len()], vec![vec![Some(0.0); n]; groups.len()], vec![vec![None; n]; groups.len()], vec![vec![Some(0.0); n]; groups.len()]];
    for p in points {
        let (Some(gi), Ok(ti)) = (index.get(p.group.as_str()), usize::try_from((p.t - first) / step.max(1))) else { continue };
        if ti >= n {
            continue;
        }
        let add = |v: &mut Option<f64>, x: f64| *v = Some(v.unwrap_or(0.0) + x);
        add(&mut out[0][*gi][ti], p.requests);
        add(&mut out[1][*gi][ti], p.tokens);
        add(&mut out[3][*gi][ti], p.cost);
        if p.requests > 0.0 && p.p95.is_finite() {
            let cur = &mut out[2][*gi][ti];
            *cur = Some(cur.unwrap_or(0.0).max(round(p.p95, 2)));
        }
    }
    let name = |g: &str| if g.is_empty() { "N/A".to_string() } else { g.to_string() };
    out.map(|vals| groups.iter().zip(vals).map(|(g, values)| LlmSeries { group: name(&g.0), values: values.into_iter().map(|v| v.map(|x| round(x, 6))).collect() }).collect())
}

// ================================================================ facets

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmFacetsRequest {
    #[serde(flatten)]
    pub q: LlmQuery,
    #[serde(default)]
    pub dimensions: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct LlmFacets {
    /// Dimension → top values with the number of LLM spans (each ignoring its own selection).
    pub facets: BTreeMap<String, Vec<SpanFacetValue>>,
}

#[utoipa::path(post, path = "/api/v1/llm/facets", operation_id = "llm_facets", request_body = LlmFacetsRequest, responses((status = 200, body = LlmFacets)), tag = "llm")]
pub async fn facets(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LlmFacetsRequest>) -> ApiResult<LlmFacets> {
    #[derive(clickhouse::Row, Deserialize)]
    struct V {
        value: String,
        count: u64,
    }
    let dims: Vec<String> = if req.dimensions.is_empty() { dimensions().into_iter().map(|d| d.0.to_string()).collect() } else { req.dimensions.clone() };
    let mut jobs = Vec::new();
    for d in dims {
        let expr = dimension(&d)?;
        let (w, params) = where_clause(&req.q, Some(&d))?;
        let st = st.clone();
        let org = org.clone();
        jobs.push(async move {
            let sql =
                format!("SELECT toString({expr}) AS value, count() AS count FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND value != '' GROUP BY value ORDER BY count DESC LIMIT 50");
            let rows = bind(st.ch(&org).query(&sql), &params).fetch_all::<V>().await?;
            Ok::<_, ApiError>((d, rows.into_iter().map(|v| SpanFacetValue { value: v.value, count: v.count }).collect::<Vec<_>>()))
        });
    }
    Ok(Json(LlmFacets { facets: futures::future::try_join_all(jobs).await?.into_iter().collect() }))
}

// ================================================================ trace list

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmTraceSearchRequest {
    #[serde(flatten)]
    pub q: LlmQuery,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmTraceRow {
    pub trace_id: String,
    /// The top LLM span (earliest, longest on ties).
    pub span_id: String,
    /// Epoch ms.
    pub timestamp: i64,
    pub ml_app: String,
    pub service: String,
    pub env: String,
    pub name: String,
    pub kind: String,
    pub model: String,
    pub input_preview: String,
    pub output_preview: String,
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub duration_ms: f64,
    /// ok | error (any LLM span of the trace failed).
    pub status: String,
    pub llm_spans: u64,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmTracePage {
    pub traces: Vec<LlmTraceRow>,
    pub total: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct ContentRow {
    trace_id: String,
    span_id: String,
    service: String,
    ts: i64,
    attrs: String,
    ev_names: Vec<String>,
    ev_attrs: String,
}

impl ContentRow {
    fn info(&self) -> Option<LlmSpanInfo> {
        let a: HashMap<String, String> = serde_json::from_str(&self.attrs).unwrap_or_default();
        let ev: Vec<HashMap<String, String>> = serde_json::from_str(&self.ev_attrs).unwrap_or_default();
        let events: Vec<attrs::Event> = self.ev_names.iter().zip(ev.iter()).map(|(n, a)| (n.as_str(), a)).collect();
        attrs::extract(&self.service, &a, &events)
    }
}

const PREVIEW_CHARS: usize = 240;

fn preview(text: &str) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > PREVIEW_CHARS {
        format!("{}…", t.chars().take(PREVIEW_CHARS).collect::<String>())
    } else {
        t
    }
}

/// Input preview: the user's last message (else any input).
fn input_preview(m: &[LlmMessage]) -> String {
    let user: Vec<&LlmMessage> = m.iter().filter(|x| x.role == "user").collect();
    preview(user.last().map(|x| x.content.as_str()).unwrap_or(&attrs::text_of(m, &[])))
}

/// The previews of a trace: the root LLM span's, else the first call's input and the last call's output.
fn previews(root: Option<&LlmSpanInfo>, calls: &[&LlmSpanInfo]) -> (String, String) {
    let mut input = root.map(|r| input_preview(&r.input)).unwrap_or_default();
    let mut output = root.map(|r| preview(&attrs::text_of(&r.output, &[]))).unwrap_or_default();
    if input.is_empty() {
        input = calls.iter().map(|c| input_preview(&c.input)).find(|s| !s.is_empty()).unwrap_or_default();
    }
    if output.is_empty() {
        output = calls.iter().rev().map(|c| preview(&attrs::text_of(&c.output, &[]))).find(|s| !s.is_empty()).unwrap_or_default();
    }
    (input, output)
}

#[utoipa::path(post, path = "/api/v1/llm/traces/search", operation_id = "llm_search_traces", request_body = LlmTraceSearchRequest, responses((status = 200, body = LlmTracePage)), tag = "llm")]
pub async fn search_traces(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LlmTraceSearchRequest>) -> ApiResult<LlmTracePage> {
    let (base, mut params) = base_clause(&req.q.range)?;
    // A trace matches when one of its LLM spans does; the row aggregates all of its LLM spans.
    let mut scope = base.clone();
    if req.q.narrowed() {
        let (w, p) = where_clause(&req.q, None)?;
        scope = format!("{base} AND TraceId IN (SELECT TraceId FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w})");
        params.extend(p);
    }
    let kind = attrs::kind_sql();
    let call = attrs::is_call_sql();
    let (model, app) = (attrs::first_sql(attrs::MODEL_KEYS), attrs::ml_app_sql());
    let (tin, tout) = (attrs::input_tokens_sql(), attrs::output_tokens_sql());
    #[derive(clickhouse::Row, Deserialize)]
    struct T {
        trace_id: String,
        span_id: String,
        start: i64,
        app: String,
        service: String,
        env: String,
        name: String,
        kind: String,
        duration: u64,
        model: String,
        tin: f64,
        tout: f64,
        errors: u64,
        spans: u64,
    }
    let order = "(Timestamp, 0 - toInt64(Duration))";
    let sql = format!(
        "SELECT TraceId AS trace_id, argMin(SpanId, {order}) AS span_id, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS start, argMin({app}, {order}) AS app, \
         argMin(toString(ServiceName), {order}) AS service, argMin(ResourceAttributes['deployment.environment'], {order}) AS env, argMin(toString(SpanName), {order}) AS name, \
         argMin({kind}, {order}) AS kind, argMin(Duration, {order}) AS duration, anyIf({model}, {kind} = 'llm' AND {model} != '') AS model, \
         sumIf({tin}, {call}) AS tin, sumIf({tout}, {call}) AS tout, countIf(StatusCode = 'Error') AS errors, count() AS spans \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {scope} GROUP BY trace_id ORDER BY start DESC LIMIT {} OFFSET {}",
        req.limit.clamp(1, 200),
        req.offset.clamp(0, 10_000)
    );
    let rows_q = bind(st.ch(&org).query(&sql), &params);
    let count_q = bind(st.ch(&org).query(&format!("SELECT uniqExact(TraceId) FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {scope}")), &params);
    let (rows, total) = tokio::try_join!(rows_q.fetch_all::<T>(), count_q.fetch_one::<u64>())?;
    if rows.is_empty() {
        return Ok(Json(LlmTracePage { traces: vec![], total }));
    }
    // Messages of the listed traces' LLM spans, for the previews.
    let ids: Vec<String> = rows.iter().map(|r| r.trace_id.clone()).collect();
    let content = st
        .ch(&org)
        .query(&format!(
            "SELECT TraceId AS trace_id, SpanId AS span_id, toString(ServiceName) AS service, toInt64(toUnixTimestamp64Nano(Timestamp)) AS ts, {json} AS attrs, \
             arrayMap(x -> toString(x), Events.Name) AS ev_names, toJSONString(Events.Attributes) AS ev_attrs FROM otel.otel_traces \
             WHERE OrgId = {{oc_org:String}} AND has(?, TraceId) AND Timestamp >= fromUnixTimestamp64Milli(?) - INTERVAL 5 MINUTE AND Timestamp <= fromUnixTimestamp64Milli(?) + INTERVAL 5 MINUTE \
             AND {llm} ORDER BY Timestamp LIMIT 5000",
            json = attrs::llm_attrs_json_sql(),
            llm = attrs::is_llm_sql()
        ))
        .bind(ids)
        .bind(req.q.range.from)
        .bind(req.q.range.to)
        .fetch_all::<ContentRow>()
        .await?;
    let mut by_trace: HashMap<&str, Vec<(&ContentRow, LlmSpanInfo)>> = HashMap::new();
    for c in &content {
        if let Some(info) = c.info() {
            by_trace.entry(c.trace_id.as_str()).or_default().push((c, info));
        }
    }
    let traces = rows
        .into_iter()
        .map(|r| {
            let spans = by_trace.get(r.trace_id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
            let root = spans.iter().find(|(c, _)| c.span_id == r.span_id).map(|(_, i)| i);
            let mut calls: Vec<(&ContentRow, &LlmSpanInfo)> = spans.iter().filter(|(_, i)| attrs::CALL_KINDS.contains(&i.kind.as_str())).map(|(c, i)| (*c, i)).collect();
            calls.sort_by_key(|(c, _)| c.ts);
            let call_infos: Vec<&LlmSpanInfo> = calls.iter().map(|(_, i)| *i).collect();
            let (input_preview, output_preview) = previews(root, &call_infos);
            let estimated_cost_usd = calls.iter().map(|(_, i)| attrs::estimate_cost(&i.model, i.input_tokens, i.output_tokens)).sum::<Option<f64>>().map(|c| round(c, 6));
            LlmTraceRow {
                trace_id: r.trace_id,
                span_id: r.span_id,
                timestamp: r.start,
                ml_app: r.app,
                service: r.service,
                env: r.env,
                name: r.name,
                kind: r.kind,
                model: r.model,
                input_preview,
                output_preview,
                input_tokens: r.tin,
                output_tokens: r.tout,
                duration_ms: round(r.duration as f64 / 1e6, 3),
                status: if r.errors > 0 { "error".into() } else { "ok".into() },
                llm_spans: r.spans,
                estimated_cost_usd,
            }
        })
        .collect();
    Ok(Json(LlmTracePage { traces, total }))
}

// ================================================================ one trace

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmSpanDetail {
    #[serde(flatten)]
    pub info: LlmSpanInfo,
    pub estimated_cost_usd: Option<f64>,
    /// Built-in checks that need no population (outliers are on the Evaluations page).
    pub checks: Vec<CheckOutcome>,
}

#[derive(Serialize, ToSchema)]
pub struct LlmTraceDetail {
    pub trace: TraceDetail,
    /// Span id → LLM details, for the LLM spans of the trace.
    pub llm: HashMap<String, LlmSpanDetail>,
}

fn sample_of(info: &LlmSpanInfo, duration_ms: f64, error: bool) -> CallSample {
    CallSample {
        model: info.model.clone(),
        duration_ms,
        error,
        input: attrs::text_of(&info.input, &["user", "tool"]),
        output: attrs::text_of(&info.output, &[]),
        total_tokens: info.input_tokens + info.output_tokens,
    }
}

fn span_detail(info: LlmSpanInfo, duration_ms: f64, error: bool) -> LlmSpanDetail {
    let is_call = info.kind == "llm";
    let checks = if is_call { checks::evaluate(&sample_of(&info, duration_ms, error), &HashMap::new(), &HashMap::new()) } else { vec![] };
    LlmSpanDetail { estimated_cost_usd: attrs::estimate_cost(&info.model, info.input_tokens, info.output_tokens).map(|c| round(c, 6)), info, checks }
}

#[utoipa::path(get, path = "/api/v1/llm/traces/{trace_id}", operation_id = "llm_trace", params(("trace_id" = String, Path, description = "32 hex characters")), responses((status = 200, body = LlmTraceDetail)), tag = "llm")]
pub async fn trace(State(st): State<AppState>, OrgId(org): OrgId, Path(trace_id): Path<String>) -> ApiResult<LlmTraceDetail> {
    let Json(trace) = super::apm::trace(State(st), OrgId(org), Path(trace_id)).await?;
    let llm = trace
        .spans
        .iter()
        .filter_map(|s| {
            let events: Vec<attrs::Event> = s.events.iter().map(|e| (e.name.as_str(), &e.attributes)).collect();
            let info = attrs::extract(&s.service, &s.attributes, &events)?;
            Some((s.span_id.clone(), span_detail(info, s.duration_us / 1000.0, s.status == "Error")))
        })
        .collect();
    Ok(Json(LlmTraceDetail { trace, llm }))
}

// ================================================================ evaluations

/// Newest LLM calls evaluated per request.
const EVAL_SAMPLE: i64 = 2000;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmEvaluationsRequest {
    #[serde(flatten)]
    pub q: LlmQuery,
    /// Only results of this check (a built-in id or a recorded evaluation name).
    pub check: Option<String>,
    /// fail (default) | pass | all
    pub outcome: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmCheckSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    /// built_in | recorded (evaluation results found on the spans)
    pub source: String,
    /// A keyword / pattern heuristic rather than an exact rule.
    pub heuristic: bool,
    pub passed: u64,
    pub failed: u64,
    /// Recorded results whose label is neither a pass nor a fail word.
    pub unlabeled: u64,
    /// Pass and fail counts per bucket.
    pub pass_series: Vec<u64>,
    pub fail_series: Vec<u64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmEvaluationResult {
    pub trace_id: String,
    pub span_id: String,
    pub timestamp: i64,
    pub ml_app: String,
    pub model: String,
    pub span_name: String,
    pub check: String,
    pub passed: Option<bool>,
    pub detail: String,
    pub score: Option<f64>,
    pub input_preview: String,
    pub output_preview: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmEvaluations {
    /// LLM calls evaluated (the newest ones, up to the sample size).
    pub evaluated: u64,
    /// LLM calls matching the query in the range.
    pub total_calls: u64,
    pub step_ms: i64,
    pub times: Vec<i64>,
    pub checks: Vec<LlmCheckSummary>,
    /// Matching results, newest first (at most 200).
    pub results: Vec<LlmEvaluationResult>,
}

#[derive(clickhouse::Row, Deserialize)]
struct EvalRow {
    trace_id: String,
    span_id: String,
    service: String,
    name: String,
    ts: i64,
    duration: u64,
    error: bool,
    attrs: String,
    ev_names: Vec<String>,
    ev_attrs: String,
}

#[utoipa::path(post, path = "/api/v1/llm/evaluations", operation_id = "llm_evaluations", request_body = LlmEvaluationsRequest, responses((status = 200, body = LlmEvaluations)), tag = "llm")]
pub async fn evaluations(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<LlmEvaluationsRequest>) -> ApiResult<LlmEvaluations> {
    let (w, params) = where_clause(&req.q, None)?;
    let scope = format!("{w} AND ({} = 'llm' OR {})", attrs::kind_sql(), attrs::has_evaluation_sql());
    let rows_q = bind(
        st.ch(&org).query(&format!(
            "SELECT TraceId AS trace_id, SpanId AS span_id, toString(ServiceName) AS service, toString(SpanName) AS name, toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, Duration AS duration, \
             StatusCode = 'Error' AS error, {json} AS attrs, arrayMap(x -> toString(x), Events.Name) AS ev_names, toJSONString(Events.Attributes) AS ev_attrs \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {scope} ORDER BY Timestamp DESC LIMIT {EVAL_SAMPLE}",
            json = attrs::llm_attrs_json_sql()
        )),
        &params,
    );
    let count_q = bind(st.ch(&org).query(&format!("SELECT countIf({} = 'llm') FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w}", attrs::kind_sql())), &params);
    let (rows, total_calls) = tokio::try_join!(rows_q.fetch_all::<EvalRow>(), count_q.fetch_one::<u64>())?;
    let step = step_for(&req.q.range, 48);
    let times = bucket_times(&req.q.range, step);
    let spans: Vec<(EvalRow, LlmSpanInfo)> = rows
        .into_iter()
        .filter_map(|r| {
            let a: HashMap<String, String> = serde_json::from_str(&r.attrs).unwrap_or_default();
            let ev: Vec<HashMap<String, String>> = serde_json::from_str(&r.ev_attrs).unwrap_or_default();
            let events: Vec<attrs::Event> = r.ev_names.iter().zip(ev.iter()).map(|(n, a)| (n.as_str(), a)).collect();
            let info = attrs::extract(&r.service, &a, &events)?;
            Some((r, info))
        })
        .collect();
    Ok(Json(evaluate_spans(&spans, times, step, req.check.as_deref(), req.outcome.as_deref().unwrap_or("fail"), total_calls)))
}

/// Runs the built-in checks over the calls and collects recorded evaluations; summaries per
/// check, series per bucket and the matching results.
fn evaluate_spans(spans: &[(EvalRow, LlmSpanInfo)], times: Vec<i64>, step: i64, check: Option<&str>, outcome: &str, total_calls: u64) -> LlmEvaluations {
    let calls: Vec<(usize, CallSample)> = spans.iter().enumerate().filter(|(_, (_, i))| i.kind == "llm").map(|(n, (r, i))| (n, sample_of(i, r.duration as f64 / 1e6, r.error))).collect();
    let samples: Vec<CallSample> = calls.iter().map(|(_, s)| s.clone()).collect();
    let tf = checks::fences(&samples, |s| s.total_tokens);
    let lf = checks::fences(&samples, |s| s.duration_ms);
    let n = times.len();
    let first = times.first().copied().unwrap_or(0);
    let mut summaries: Vec<LlmCheckSummary> = checks::CHECKS
        .iter()
        .map(|(id, name, desc, heuristic)| LlmCheckSummary {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            source: "built_in".into(),
            heuristic: *heuristic,
            passed: 0,
            failed: 0,
            unlabeled: 0,
            pass_series: vec![0; n],
            fail_series: vec![0; n],
        })
        .collect();
    let mut results = Vec::new();
    let mut record = |summaries: &mut Vec<LlmCheckSummary>, id: &str, r: &EvalRow, info: &LlmSpanInfo, passed: Option<bool>, detail: String, score: Option<f64>| {
        let idx = match summaries.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => {
                summaries.push(LlmCheckSummary {
                    id: id.to_string(),
                    name: id.to_string(),
                    description: "Recorded on the spans by the application or an evaluator (gen_ai.evaluation.*).".into(),
                    source: "recorded".into(),
                    heuristic: false,
                    passed: 0,
                    failed: 0,
                    unlabeled: 0,
                    pass_series: vec![0; n],
                    fail_series: vec![0; n],
                });
                summaries.len() - 1
            }
        };
        let s = &mut summaries[idx];
        let bucket = usize::try_from((r.ts - first) / step.max(1)).ok().filter(|b| *b < n);
        match passed {
            Some(true) => {
                s.passed += 1;
                if let Some(b) = bucket {
                    s.pass_series[b] += 1;
                }
            }
            Some(false) => {
                s.failed += 1;
                if let Some(b) = bucket {
                    s.fail_series[b] += 1;
                }
            }
            None => s.unlabeled += 1,
        }
        let wanted = check.is_none_or(|c| c == id)
            && match outcome {
                "all" => true,
                "pass" => passed == Some(true),
                _ => passed == Some(false),
            };
        if wanted && results.len() < 200 {
            results.push(LlmEvaluationResult {
                trace_id: r.trace_id.clone(),
                span_id: r.span_id.clone(),
                timestamp: r.ts,
                ml_app: info.ml_app.clone(),
                model: info.model.clone(),
                span_name: r.name.clone(),
                check: id.to_string(),
                passed,
                detail,
                score,
                input_preview: input_preview(&info.input),
                output_preview: preview(&attrs::text_of(&info.output, &[])),
            });
        }
    };
    for (n, sample) in &calls {
        let (r, info) = &spans[*n];
        for o in checks::evaluate(sample, &tf, &lf) {
            record(&mut summaries, &o.check, r, info, Some(o.passed), o.detail, None);
        }
    }
    for (r, info) in spans {
        for e in &info.evaluations {
            let detail = [e.label.as_str(), e.explanation.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(": ");
            record(&mut summaries, &e.name, r, info, e.passed, detail, e.score);
        }
    }
    LlmEvaluations { evaluated: calls.len() as u64, total_calls, step_ms: step, times, checks: summaries, results }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(query: &str, filters: &[(&str, &[&str])]) -> LlmQuery {
        LlmQuery { range: TimeRange { from: 1_000, to: 61_000 }, query: query.into(), filters: filters.iter().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())).collect() }
    }

    #[test]
    fn where_clause_binds_every_value() {
        let (w, params) = where_clause(&q("service:support-assistant", &[("model", &["gpt-4o", "x' OR 1=1"]), ("status", &["error"])]), None).unwrap();
        assert!(w.starts_with("Timestamp >= fromUnixTimestamp64Milli(?)"));
        assert!(w.contains(&attrs::is_llm_sql()));
        assert!(!w.contains("x' OR 1=1"), "values are bound, not interpolated");
        assert!(w.contains(&format!("{} IN (?, ?)", attrs::first_sql(attrs::MODEL_KEYS))));
        assert!(w.contains("if(StatusCode = 'Error', 'error', 'ok') IN (?)"));
        assert_eq!(params.len(), 2 + 1 + 3);
        let (skip, _) = where_clause(&q("", &[("model", &["gpt-4o"])]), Some("model")).unwrap();
        assert!(!skip.contains(" IN (?)"), "a facet ignores its own selection");
        assert!(where_clause(&q("", &[("nope", &["x"])]), None).is_err());
    }

    #[test]
    fn series_keep_the_busiest_groups() {
        let p = |g: &str, t: i64, requests: f64, tokens: f64, p95: f64, cost: f64| SeriesPoint { group: g.into(), t, requests, tokens, p95, cost };
        let points = vec![
            p("support-assistant", 0, 3.0, 300.0, 120.0, 0.5),
            p("support-assistant", 0, 1.0, 50.0, 400.0, 0.25),
            p("support-assistant", 20, 1.0, 50.0, 90.0, 0.0),
            p("", 10, 9.0, 10.0, 5.0, 0.0),
        ];
        let [req, tok, lat, cost] = series(&[0, 10, 20], 10, points);
        assert_eq!(req[0].group, "N/A", "most requests first");
        assert_eq!(req[1].values, vec![Some(4.0), Some(0.0), Some(1.0)], "models of one app add up");
        assert_eq!(tok[1].values, vec![Some(350.0), Some(0.0), Some(50.0)]);
        assert_eq!(lat[1].values, vec![Some(400.0), None, Some(90.0)], "the slowest model's p95");
        assert_eq!(cost[1].values, vec![Some(0.75), Some(0.0), Some(0.0)]);
    }

    #[test]
    fn usage_rolls_up_by_model_and_app() {
        let u = |app: &str, model: &str, requests: u64, tin: f64, tout: f64| Usage { app: app.into(), model: model.into(), provider: "openai".into(), requests, errors: 1, tin, tout };
        let rows = vec![u("support-assistant", "gpt-4o", 10, 1_000_000.0, 100_000.0), u("product-search", "gpt-4o", 5, 0.0, 0.0), u("support-assistant", "my-finetune", 2, 10.0, 10.0)];
        let lat = HashMap::from([("support-assistant".to_string(), (800.0, 1900.0))]);
        let (models, apps, unpriced) = summarize(rows, &lat);
        assert_eq!(models[0].model, "gpt-4o");
        assert_eq!((models[0].requests, models[0].errors), (15, 2));
        assert_eq!(models[0].estimated_cost_usd, Some(3.5));
        assert_eq!(models[1].estimated_cost_usd, None);
        assert_eq!(unpriced, vec!["my-finetune"]);
        assert_eq!(apps[0].ml_app, "support-assistant");
        assert_eq!((apps[0].requests, apps[0].p95_ms, apps[0].estimated_cost_usd), (12, 1900.0, 3.5));
        assert_eq!(apps[0].models, vec!["gpt-4o", "my-finetune"]);
        assert_eq!(apps[1].p95_ms, 0.0);
    }

    #[test]
    fn previews_fall_back_to_the_model_calls() {
        let msg = |role: &str, c: &str| LlmMessage { role: role.into(), content: c.into() };
        let root = LlmSpanInfo { kind: "agent".into(), ..Default::default() };
        let first = LlmSpanInfo { kind: "llm".into(), input: vec![msg("system", "rules"), msg("user", "Where is   order 7?")], output: vec![msg("assistant", "tool call")], ..Default::default() };
        let last = LlmSpanInfo { kind: "llm".into(), output: vec![msg("assistant", "It ships today.")], ..Default::default() };
        assert_eq!(previews(Some(&root), &[&first, &last]), ("Where is order 7?".to_string(), "It ships today.".to_string()));
        assert_eq!(preview(&"x".repeat(300)).chars().count(), PREVIEW_CHARS + 1);
    }

    fn row(span: &str, ts: i64, error: bool) -> EvalRow {
        EvalRow {
            trace_id: "t".repeat(32),
            span_id: span.into(),
            service: "support-assistant".into(),
            name: "chat gpt-4o-mini".into(),
            ts,
            duration: 900_000_000,
            error,
            attrs: "{}".into(),
            ev_names: vec![],
            ev_attrs: "[]".into(),
        }
    }

    #[test]
    fn evaluations_count_built_in_and_recorded_results() {
        let msg = |role: &str, c: &str| LlmMessage { role: role.into(), content: c.into() };
        let ok = LlmSpanInfo {
            kind: "llm".into(),
            ml_app: "support-assistant".into(),
            model: "gpt-4o-mini".into(),
            input: vec![msg("user", "hi")],
            output: vec![msg("assistant", "Hello!")],
            ..Default::default()
        };
        let leaky = LlmSpanInfo { output: vec![msg("assistant", "Mail jane@example.com")], ..ok.clone() };
        let rated = LlmSpanInfo {
            evaluations: vec![attrs::RecordedEvaluation { name: "helpfulness".into(), score: Some(0.2), label: "fail".into(), explanation: "vague".into(), passed: Some(false) }],
            ..ok.clone()
        };
        let spans = vec![(row("a", 1_000, false), ok.clone()), (row("b", 11_000, false), leaky), (row("c", 21_000, true), ok), (row("d", 21_500, false), rated)];
        let out = evaluate_spans(&spans, vec![0, 10_000, 20_000], 10_000, None, "fail", 4);
        assert_eq!(out.evaluated, 4);
        let by = |id: &str| out.checks.iter().find(|c| c.id == id).unwrap();
        assert_eq!((by("error").passed, by("error").failed), (3, 1));
        assert_eq!(by("error").fail_series, vec![0, 0, 1]);
        assert_eq!((by("pii_in_output").passed, by("pii_in_output").failed), (2, 1));
        assert!(by("pii_in_output").heuristic);
        let rec = by("helpfulness");
        assert_eq!((rec.source.as_str(), rec.failed), ("recorded", 1));
        let failed: Vec<(&str, &str)> = out.results.iter().map(|r| (r.span_id.as_str(), r.check.as_str())).collect();
        assert_eq!(failed, vec![("b", "pii_in_output"), ("c", "error"), ("d", "helpfulness")]);
        assert_eq!(out.results[2].detail, "fail: vague");

        let only = evaluate_spans(&spans, vec![0, 10_000, 20_000], 10_000, Some("error"), "pass", 4);
        assert_eq!(only.results.len(), 3);
    }
}
