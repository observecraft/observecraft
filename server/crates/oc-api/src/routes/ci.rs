//! Software Delivery: CI/CD pipeline executions, flaky tests and DORA metrics.
//!
//! Pipeline runs arrive as OpenTelemetry traces (the Collector routes any trace whose resource has
//! `cicd.pipeline.name` into `otel.otel_ci_traces`): the root span is the pipeline, its children
//! are jobs (`cicd.pipeline.task.name`), then steps (`ci.step.name`) and test cases
//! (`test.case.name`). Run, repository and commit details are resource attributes (`cicd.*`, `vcs.*`).
//! Deploy jobs carry `deployment.environment.name`; DORA lead time uses `vcs.revision.committed_at`.
//!
//! Tenancy: like the other OTel tables, `otel_ci_traces` has no org column yet (single-tenant).

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const TABLE: &str = "otel.otel_ci_traces";
const PIPELINE: &str = "ResourceAttributes['cicd.pipeline.name']";
const REPO: &str = "ResourceAttributes['vcs.repository.name']";
const BRANCH: &str = "ResourceAttributes['vcs.ref.head.name']";
const SHA: &str = "ResourceAttributes['vcs.ref.head.revision']";
const PROVIDER: &str = "ResourceAttributes['cicd.provider.name']";
const TEAM: &str = "ResourceAttributes['team']";
const JOB: &str = "SpanAttributes['cicd.pipeline.task.name']";
const STAGE: &str = "SpanAttributes['ci.stage.name']";
const TEST: &str = "SpanAttributes['test.case.name']";
/// success | error | canceled | skipped, from the pipeline or job result (falling back to the span status).
pub(crate) const STATUS: &str = "multiIf(\
    if(ParentSpanId = '', SpanAttributes['cicd.pipeline.result'], SpanAttributes['cicd.pipeline.task.run.result']) IN ('success', 'succeeded', 'pass'), 'success', \
    if(ParentSpanId = '', SpanAttributes['cicd.pipeline.result'], SpanAttributes['cicd.pipeline.task.run.result']) IN ('cancellation', 'canceled', 'cancelled'), 'canceled', \
    if(ParentSpanId = '', SpanAttributes['cicd.pipeline.result'], SpanAttributes['cicd.pipeline.task.run.result']) IN ('skip', 'skipped'), 'skipped', \
    StatusCode = 'Error' OR if(ParentSpanId = '', SpanAttributes['cicd.pipeline.result'], SpanAttributes['cicd.pipeline.task.run.result']) != '', 'error', 'success')";

/// Query parameters in text order (strings, integers and string arrays).
enum P {
    S(String),
    I(i64),
    A(Vec<String>),
}

fn bind(mut q: clickhouse::query::Query, params: &[P]) -> clickhouse::query::Query {
    for p in params {
        q = match p {
            P::S(s) => q.bind(s.as_str()),
            P::I(i) => q.bind(*i),
            P::A(a) => q.bind(a.clone()),
        };
    }
    q
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CiLevel {
    #[default]
    Pipeline,
    Job,
}

impl CiLevel {
    fn clause(self) -> &'static str {
        match self {
            CiLevel::Pipeline => "ParentSpanId = ''",
            CiLevel::Job => "SpanAttributes['cicd.pipeline.task.name'] != ''",
        }
    }
}

#[derive(Deserialize, ToSchema, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct CiFilters {
    /// success | error | canceled | skipped
    pub statuses: Vec<String>,
    pub pipelines: Vec<String>,
    pub repositories: Vec<String>,
    pub branches: Vec<String>,
    pub providers: Vec<String>,
    pub teams: Vec<String>,
}

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CiQuery {
    pub range: TimeRange,
    #[serde(default)]
    pub level: CiLevel,
    /// Words match pipeline/job names and repositories; `key:value` terms filter on status,
    /// pipeline, repository (repo), branch, provider, team, sha, job, stage or author.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub filters: CiFilters,
}

#[derive(Clone, Copy, PartialEq)]
enum Facet {
    Status,
    Pipeline,
    Repository,
    Branch,
    Provider,
    Team,
}

/// WHERE clause for a query, optionally ignoring one facet's filter (for that facet's counts).
fn where_clause(q: &CiQuery, skip: Option<Facet>) -> Result<(String, Vec<P>), ApiError> {
    q.range.validate()?;
    let mut parts = vec![
        "Timestamp >= fromUnixTimestamp64Milli(?)".to_string(),
        "Timestamp <= fromUnixTimestamp64Milli(?)".to_string(),
        q.level.clause().to_string(),
    ];
    let mut params = vec![P::I(q.range.from), P::I(q.range.to)];
    let f = &q.filters;
    let lists: [(Facet, &str, &Vec<String>); 6] = [
        (Facet::Status, STATUS, &f.statuses),
        (Facet::Pipeline, PIPELINE, &f.pipelines),
        (Facet::Repository, REPO, &f.repositories),
        (Facet::Branch, BRANCH, &f.branches),
        (Facet::Provider, PROVIDER, &f.providers),
        (Facet::Team, TEAM, &f.teams),
    ];
    for (facet, expr, values) in lists {
        if skip != Some(facet) && !values.is_empty() {
            parts.push(format!("has(?, {expr})"));
            params.push(P::A(values.clone()));
        }
    }
    for word in q.query.split_whitespace() {
        let (neg, word) = word.strip_prefix('-').map(|w| (true, w)).unwrap_or((false, word));
        let clause = match word.split_once(':') {
            Some((key, value)) if !value.is_empty() => {
                let expr = match key {
                    "status" | "@ci.status" => STATUS,
                    "pipeline" | "@ci.pipeline.name" => PIPELINE,
                    "repository" | "repo" | "@git.repository.name" => REPO,
                    "branch" | "@git.branch" => BRANCH,
                    "provider" | "@ci.provider.name" => PROVIDER,
                    "team" => TEAM,
                    "sha" | "commit" | "@git.commit.sha" => SHA,
                    "job" | "@ci.job.name" => JOB,
                    "stage" | "@ci.stage.name" => STAGE,
                    "author" => "ResourceAttributes['vcs.revision.author']",
                    other => return Err(ApiError::BadRequest(format!("unknown filter '{other}' (use status, pipeline, repo, branch, provider, team, sha, job, stage or author)"))),
                };
                if value.contains('*') {
                    params.push(P::S(value.replace('%', "\\%").replace('_', "\\_").replace('*', "%")));
                    format!("{expr} ILIKE ?")
                } else {
                    params.push(P::S(value.to_string()));
                    format!("{expr} = ?")
                }
            }
            _ => {
                let like = format!("%{}%", word.replace('%', "\\%").replace('_', "\\_"));
                params.push(P::S(like.clone()));
                params.push(P::S(like));
                format!("(SpanName ILIKE ? OR {REPO} ILIKE ?)")
            }
        };
        parts.push(if neg { format!("NOT ({clause})") } else { clause });
    }
    Ok((parts.join(" AND "), params))
}

// ================================================================ executions

#[derive(Serialize, ToSchema, clickhouse::Row, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiExecution {
    pub trace_id: String,
    pub span_id: String,
    /// Pipeline name (pipeline level) or job name (job level).
    pub name: String,
    pub pipeline_name: String,
    /// success | error | canceled | skipped
    pub status: String,
    /// Epoch ms.
    pub start: i64,
    pub duration_ms: f64,
    pub run_id: String,
    pub run_url: String,
    pub attempt: String,
    pub repository: String,
    pub repository_url: String,
    pub branch: String,
    pub sha: String,
    pub author: String,
    pub message: String,
    pub provider: String,
    /// Job level only.
    pub stage: String,
    pub team: String,
}

const ROW: &str = "TraceId AS trace_id, SpanId AS span_id, SpanName AS name, ResourceAttributes['cicd.pipeline.name'] AS pipeline_name, {STATUS} AS status, \
    toInt64(toUnixTimestamp64Milli(Timestamp)) AS start, Duration / 1e6 AS duration_ms, ResourceAttributes['cicd.pipeline.run.id'] AS run_id, \
    ResourceAttributes['cicd.pipeline.run.url.full'] AS run_url, ResourceAttributes['cicd.pipeline.run.attempt'] AS attempt, \
    ResourceAttributes['vcs.repository.name'] AS repository, ResourceAttributes['vcs.repository.url.full'] AS repository_url, \
    ResourceAttributes['vcs.ref.head.name'] AS branch, ResourceAttributes['vcs.ref.head.revision'] AS sha, ResourceAttributes['vcs.revision.author'] AS author, \
    ResourceAttributes['vcs.revision.message'] AS message, ResourceAttributes['cicd.provider.name'] AS provider, SpanAttributes['ci.stage.name'] AS stage, \
    ResourceAttributes['team'] AS team";

fn row_select() -> String {
    ROW.replace("{STATUS}", STATUS)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CiExecutionSearch {
    #[serde(flatten)]
    pub q: CiQuery,
    /// date (default) | duration
    pub sort: Option<String>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
pub struct CiExecutionPage {
    pub executions: Vec<CiExecution>,
    pub total: u64,
}

#[utoipa::path(post, operation_id = "ci_executions", path = "/api/v1/ci/executions", request_body = CiExecutionSearch, responses((status = 200, body = CiExecutionPage)), tag = "ci")]
pub async fn executions(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CiExecutionSearch>) -> ApiResult<CiExecutionPage> {
    let (w, params) = where_clause(&req.q, None)?;
    let order = if req.sort.as_deref() == Some("duration") { "Duration DESC" } else { "Timestamp DESC" };
    let sql = format!("SELECT {} FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {w} ORDER BY {order} LIMIT {} OFFSET {}", row_select(), req.limit.clamp(1, 500), req.offset.clamp(0, 100_000));
    let rows = bind(st.ch(&org).query(&sql), &params).fetch_all::<CiExecution>().await?;
    let total = bind(st.ch(&org).query(&format!("SELECT count() FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {w}")), &params).fetch_one::<u64>().await?;
    Ok(Json(CiExecutionPage { executions: rows, total }))
}

#[derive(Serialize, ToSchema, Clone)]
pub struct CiFacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
pub struct CiFacets {
    pub status: Vec<CiFacetValue>,
    pub pipeline: Vec<CiFacetValue>,
    pub repository: Vec<CiFacetValue>,
    pub branch: Vec<CiFacetValue>,
    pub provider: Vec<CiFacetValue>,
    pub team: Vec<CiFacetValue>,
}

#[utoipa::path(post, operation_id = "ci_facets", path = "/api/v1/ci/executions/facets", request_body = CiQuery, responses((status = 200, body = CiFacets)), tag = "ci")]
pub async fn facets(State(st): State<AppState>, OrgId(org): OrgId, Json(q): Json<CiQuery>) -> ApiResult<CiFacets> {
    let org = org.as_str();
    #[derive(clickhouse::Row, Deserialize)]
    struct V {
        value: String,
        count: u64,
    }
    let one = |facet: Facet, expr: &'static str| {
        let st = st.clone();
        let q = q.clone();
        async move {
            let (w, params) = where_clause(&q, Some(facet))?;
            let sql = format!("SELECT toString({expr}) AS value, count() AS count FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {w} AND value != '' GROUP BY value ORDER BY count DESC LIMIT 100");
            let rows = bind(st.ch(org).query(&sql), &params).fetch_all::<V>().await?;
            Ok::<_, ApiError>(rows.into_iter().map(|v| CiFacetValue { value: v.value, count: v.count }).collect::<Vec<_>>())
        }
    };
    let (status, pipeline, repository, branch, provider, team) = futures::try_join!(
        one(Facet::Status, STATUS),
        one(Facet::Pipeline, PIPELINE),
        one(Facet::Repository, REPO),
        one(Facet::Branch, BRANCH),
        one(Facet::Provider, PROVIDER),
        one(Facet::Team, TEAM),
    )?;
    Ok(Json(CiFacets { status, pipeline, repository, branch, provider, team }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CiChartSeries {
    pub name: String,
    /// Average duration (ms) per bucket; null when no execution.
    pub duration: Vec<Option<f64>>,
    pub errored: Vec<u64>,
    pub executions: Vec<u64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CiCharts {
    pub step_ms: i64,
    /// Bucket starts (epoch ms).
    pub times: Vec<i64>,
    /// One series per pipeline (job level: per job), the 8 busiest; the rest are merged into "other".
    pub series: Vec<CiChartSeries>,
}

fn step_for(range: TimeRange, buckets: i64) -> i64 {
    let raw = (range.to - range.from) / buckets;
    const STEPS: [i64; 10] = [60_000, 300_000, 600_000, 1_800_000, 3_600_000, 3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, 86_400_000, 7 * 86_400_000];
    STEPS.iter().copied().find(|s| *s >= raw).unwrap_or(7 * 86_400_000)
}

#[utoipa::path(post, operation_id = "ci_charts", path = "/api/v1/ci/executions/charts", request_body = CiQuery, responses((status = 200, body = CiCharts)), tag = "ci")]
pub async fn charts(State(st): State<AppState>, OrgId(org): OrgId, Json(q): Json<CiQuery>) -> ApiResult<CiCharts> {
    let (w, params) = where_clause(&q, None)?;
    let step = step_for(q.range, 60);
    #[derive(clickhouse::Row, Deserialize)]
    struct R {
        name: String,
        t: i64,
        dur: f64,
        errored: u64,
        n: u64,
    }
    let sql = format!(
        "SELECT SpanName AS name, intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, avg(Duration) / 1e6 AS dur, \
         countIf({STATUS} = 'error') AS errored, count() AS n FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY name, t ORDER BY t"
    );
    let rows = bind(st.ch(&org).query(&sql), &params).fetch_all::<R>().await?;
    let first = q.range.from / step * step;
    let times: Vec<i64> = (0..).map(|i| first + i * step).take_while(|t| *t <= q.range.to).collect();
    let index: HashMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let mut totals: HashMap<String, u64> = HashMap::new();
    for r in &rows {
        *totals.entry(r.name.clone()).or_default() += r.n;
    }
    let mut names: Vec<(String, u64)> = totals.into_iter().collect();
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top: Vec<String> = names.iter().take(8).map(|n| n.0.clone()).collect();
    let mut by: BTreeMap<String, (Vec<f64>, Vec<u64>, Vec<u64>)> = BTreeMap::new();
    for r in rows {
        let Some(&i) = index.get(&r.t) else { continue };
        let key = if top.contains(&r.name) { r.name } else { "other".into() };
        let e = by.entry(key).or_insert_with(|| (vec![0.0; times.len()], vec![0; times.len()], vec![0; times.len()]));
        // Weighted sum for the average; divided below.
        e.0[i] += r.dur * r.n as f64;
        e.1[i] += r.errored;
        e.2[i] += r.n;
    }
    let mut series: Vec<CiChartSeries> = by
        .into_iter()
        .map(|(name, (dur, errored, executions))| CiChartSeries {
            duration: dur.iter().zip(&executions).map(|(d, n)| (*n > 0).then(|| (d / *n as f64).round())).collect(),
            errored,
            executions,
            name,
        })
        .collect();
    series.sort_by_key(|s| std::cmp::Reverse(s.executions.iter().sum::<u64>()));
    Ok(Json(CiCharts { step_ms: step, times, series }))
}

// ================================================================ pipeline detail

#[derive(Serialize, ToSchema, clickhouse::Row, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiSpan {
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    /// pipeline | job | step | test
    pub kind: String,
    pub status: String,
    pub start: i64,
    pub duration_ms: f64,
    pub stage: String,
    pub job_type: String,
    pub environment: String,
    pub test_suite: String,
}

#[derive(Serialize, ToSchema)]
pub struct CiPipelineDetail {
    pub pipeline: CiExecution,
    pub spans: Vec<CiSpan>,
}

#[utoipa::path(get, operation_id = "ci_pipeline", path = "/api/v1/ci/pipelines/{trace_id}", params(("trace_id" = String, Path)), responses((status = 200, body = CiPipelineDetail)), tag = "ci")]
pub async fn pipeline(State(st): State<AppState>, OrgId(org): OrgId, Path(trace_id): Path<String>) -> ApiResult<CiPipelineDetail> {
    if trace_id.len() != 32 || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("invalid trace id".into()));
    }
    // The trace-id lookup table narrows the time range so the main table read stays small.
    let bounds = "(SELECT min(Start) - INTERVAL 1 HOUR FROM otel.otel_ci_traces_trace_id_ts WHERE OrgId = {oc_org:String} AND TraceId = ?) \
                  AND (SELECT max(End) + INTERVAL 1 HOUR FROM otel.otel_ci_traces_trace_id_ts WHERE OrgId = {oc_org:String} AND TraceId = ?)";
    let p = st
        .ch(&org)
        .query(&format!("SELECT {} FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND TraceId = ? AND ParentSpanId = '' AND Timestamp BETWEEN {bounds} LIMIT 1", row_select()))
        .bind(&trace_id)
        .bind(&trace_id)
        .bind(&trace_id)
        .fetch_optional::<CiExecution>()
        .await?
        .ok_or_else(|| ApiError::NotFound("pipeline execution not found".into()))?;
    let spans = st
        .ch(&org)
        .query(&format!(
            "SELECT SpanId AS span_id, ParentSpanId AS parent_span_id, SpanName AS name, \
             multiIf(ParentSpanId = '', 'pipeline', {JOB} != '', 'job', {TEST} != '', 'test', 'step') AS kind, \
             if({TEST} != '', if(SpanAttributes['test.case.result.status'] = 'fail', 'error', 'success'), {STATUS}) AS status, \
             toInt64(toUnixTimestamp64Milli(Timestamp)) AS start, Duration / 1e6 AS duration_ms, {STAGE} AS stage, SpanAttributes['cicd.pipeline.task.type'] AS job_type, \
             SpanAttributes['deployment.environment.name'] AS environment, SpanAttributes['test.suite.name'] AS test_suite \
             FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND TraceId = ? AND Timestamp BETWEEN {bounds} ORDER BY Timestamp LIMIT 5000"
        ))
        .bind(&trace_id)
        .bind(&trace_id)
        .bind(&trace_id)
        .fetch_all::<CiSpan>()
        .await?;
    Ok(Json(CiPipelineDetail { pipeline: p, spans }))
}

// ================================================================ aggregations

#[derive(Serialize, ToSchema, clickhouse::Row, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiAggregate {
    pub name: String,
    pub repository: String,
    pub executions: u64,
    pub failures: u64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub last_status: String,
    pub last_start: i64,
    pub last_trace_id: String,
}

#[utoipa::path(post, operation_id = "ci_aggregations", path = "/api/v1/ci/aggregations", request_body = CiQuery, responses((status = 200, body = Vec<CiAggregate>)), tag = "ci")]
pub async fn aggregations(State(st): State<AppState>, OrgId(org): OrgId, Json(q): Json<CiQuery>) -> ApiResult<Vec<CiAggregate>> {
    let (w, params) = where_clause(&q, None)?;
    let sql = format!(
        "SELECT SpanName AS name, {REPO} AS repository, count() AS executions, countIf({STATUS} = 'error') AS failures, avg(Duration) / 1e6 AS avg_ms, \
         quantile(0.5)(Duration) / 1e6 AS p50_ms, quantile(0.95)(Duration) / 1e6 AS p95_ms, argMax({STATUS}, Timestamp) AS last_status, \
         toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_start, argMax(TraceId, Timestamp) AS last_trace_id \
         FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY name, repository ORDER BY executions DESC LIMIT 500"
    );
    Ok(Json(bind(st.ch(&org).query(&sql), &params).fetch_all::<CiAggregate>().await?))
}

// ================================================================ overview

#[derive(Serialize, ToSchema, Default, clickhouse::Row, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiKpis {
    pub executions: u64,
    pub failures: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub repositories: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CiOverview {
    pub current: CiKpis,
    /// The same figures for the preceding period of equal length.
    pub previous: CiKpis,
    pub flaky_tests: u64,
    /// Pipelines (name + repository) with the most failures.
    pub top_failing: Vec<CiAggregate>,
    /// Pipelines with the highest p95 duration.
    pub slowest: Vec<CiAggregate>,
    pub recent_failures: Vec<CiExecution>,
}

#[derive(Deserialize, ToSchema)]
pub struct CiRangeRequest {
    pub range: TimeRange,
}

#[utoipa::path(post, operation_id = "ci_overview", path = "/api/v1/ci/overview", request_body = CiRangeRequest, responses((status = 200, body = CiOverview)), tag = "ci")]
pub async fn overview(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CiRangeRequest>) -> ApiResult<CiOverview> {
    let org = org.as_str();
    req.range.validate()?;
    let span = req.range.to - req.range.from;
    let kpis = |from: i64, to: i64| {
        let st = st.clone();
        async move {
            let sql = format!(
                "SELECT count() AS executions, countIf({STATUS} = 'error') AS failures, quantile(0.5)(Duration) / 1e6 AS p50_ms, quantile(0.95)(Duration) / 1e6 AS p95_ms, \
                 uniqExact({REPO}) AS repositories FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND ParentSpanId = '' AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)"
            );
            let k = st.ch(org).query(&sql).bind(from).bind(to).fetch_one::<CiKpis>().await?;
            Ok::<_, ApiError>(CiKpis { p50_ms: finite(k.p50_ms), p95_ms: finite(k.p95_ms), ..k })
        }
    };
    let q = CiQuery { range: req.range, level: CiLevel::Pipeline, query: String::new(), filters: CiFilters::default() };
    let (current, previous, aggs) = futures::try_join!(kpis(req.range.from, req.range.to), kpis(req.range.from - span, req.range.from), async {
        aggregations(State(st.clone()), OrgId(org.to_string()), Json(q.clone())).await.map(|j| j.0)
    })?;
    let mut top_failing: Vec<CiAggregate> = aggs.iter().filter(|a| a.failures > 0).map(clone_agg).collect();
    top_failing.sort_by(|a, b| b.failures.cmp(&a.failures));
    top_failing.truncate(5);
    let mut slowest: Vec<CiAggregate> = aggs.iter().map(clone_agg).collect();
    slowest.sort_by(|a, b| b.p95_ms.total_cmp(&a.p95_ms));
    slowest.truncate(5);
    let failed_q = CiQuery { filters: CiFilters { statuses: vec!["error".into()], ..Default::default() }, ..q.clone() };
    let (w, params) = where_clause(&failed_q, None)?;
    let recent_failures = bind(st.ch(org).query(&format!("SELECT {} FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {w} ORDER BY Timestamp DESC LIMIT 6", row_select())), &params).fetch_all::<CiExecution>().await?;
    let flaky = flaky_rows(&st, org, req.range, "").await?;
    Ok(Json(CiOverview { current, previous, flaky_tests: flaky.len() as u64, top_failing, slowest, recent_failures }))
}

fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

fn clone_agg(a: &CiAggregate) -> CiAggregate {
    CiAggregate {
        name: a.name.clone(),
        repository: a.repository.clone(),
        executions: a.executions,
        failures: a.failures,
        avg_ms: a.avg_ms,
        p50_ms: a.p50_ms,
        p95_ms: a.p95_ms,
        last_status: a.last_status.clone(),
        last_start: a.last_start,
        last_trace_id: a.last_trace_id.clone(),
    }
}

// ================================================================ flaky tests

#[derive(clickhouse::Row, Deserialize)]
struct FlakyRow {
    repository: String,
    suite: String,
    name: String,
    runs: u64,
    failures: u64,
    avg_ms: f64,
    flaky_commits: u64,
    first_flake: i64,
    last_flake: i64,
    last_failed_trace: String,
    team: String,
}

/// A test is flaky when it both passed and failed on the same commit.
async fn flaky_rows(st: &AppState, org: &str, range: TimeRange, query: &str) -> Result<Vec<FlakyRow>, ApiError> {
    let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
    let sql = format!(
        "WITH tests AS ( \
           SELECT {REPO} AS repository, SpanAttributes['test.suite.name'] AS suite, {TEST} AS name, {SHA} AS sha, \
                  SpanAttributes['test.case.result.status'] = 'fail' AS failed, Timestamp, Duration, TraceId, {TEAM} AS team \
           FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {TEST} != '' AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND (? = '' OR {TEST} ILIKE ? OR SpanAttributes['test.suite.name'] ILIKE ? OR {REPO} ILIKE ?)), \
         flaky AS ( \
           SELECT repository, suite, name, sha, min(Timestamp) AS first, max(Timestamp) AS last FROM tests \
           GROUP BY repository, suite, name, sha HAVING countIf(failed) > 0 AND countIf(NOT failed) > 0) \
         SELECT t.repository AS repository, t.suite AS suite, t.name AS name, count() AS runs, countIf(t.failed) AS failures, avg(t.Duration) / 1e6 AS avg_ms, \
                any(f.commits) AS flaky_commits, toInt64(toUnixTimestamp64Milli(any(f.first))) AS first_flake, toInt64(toUnixTimestamp64Milli(any(f.last))) AS last_flake, \
                argMaxIf(t.TraceId, t.Timestamp, t.failed) AS last_failed_trace, any(t.team) AS team \
         FROM tests t INNER JOIN (SELECT repository, suite, name, count() AS commits, min(first) AS first, max(last) AS last FROM flaky GROUP BY repository, suite, name) f \
           ON t.repository = f.repository AND t.suite = f.suite AND t.name = f.name \
         GROUP BY repository, suite, name ORDER BY flaky_commits DESC, failures DESC LIMIT 500"
    );
    Ok(st.ch(org).query(&sql).bind(range.from).bind(range.to).bind(query).bind(&like).bind(&like).bind(&like).fetch_all::<FlakyRow>().await?)
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FlakyState {
    Active,
    Quarantined,
    Disabled,
    Fixed,
}

impl FlakyState {
    fn as_str(self) -> &'static str {
        match self {
            FlakyState::Active => "active",
            FlakyState::Quarantined => "quarantined",
            FlakyState::Disabled => "disabled",
            FlakyState::Fixed => "fixed",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "quarantined" => FlakyState::Quarantined,
            "disabled" => FlakyState::Disabled,
            "fixed" => FlakyState::Fixed,
            _ => FlakyState::Active,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlakyTest {
    pub repository: String,
    pub suite: String,
    pub name: String,
    pub state: FlakyState,
    pub team: String,
    pub runs: u64,
    pub failures: u64,
    pub failure_rate: f64,
    pub avg_ms: f64,
    /// Commits on which the test both passed and failed.
    pub flaky_commits: u64,
    pub first_flake: i64,
    pub last_flake: i64,
    pub last_failed_trace_id: String,
    pub state_updated_by: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlakyRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub states: Vec<FlakyState>,
}

#[derive(Serialize, ToSchema)]
pub struct FlakyPage {
    pub tests: Vec<FlakyTest>,
    /// Per state, ignoring the state filter.
    pub counts: BTreeMap<String, u64>,
}

#[utoipa::path(post, operation_id = "ci_flaky", path = "/api/v1/ci/flaky", request_body = FlakyRequest, responses((status = 200, body = FlakyPage)), tag = "ci")]
pub async fn flaky(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<FlakyRequest>) -> ApiResult<FlakyPage> {
    req.range.validate()?;
    let rows = flaky_rows(&st, &org, req.range, req.query.trim()).await?;
    let states: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT s.repository, s.suite, s.name, s.state, u.name FROM ci_test_states s LEFT JOIN users u ON u.id = s.updated_by WHERE s.org_id = $1",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let state_of: HashMap<(String, String, String), (FlakyState, Option<String>)> =
        states.into_iter().map(|(r, s, n, state, by)| ((r, s, n), (FlakyState::parse(&state), by))).collect();
    let mut counts: BTreeMap<String, u64> = ["active", "quarantined", "disabled", "fixed"].iter().map(|s| (s.to_string(), 0)).collect();
    let mut tests = Vec::new();
    for r in rows {
        let (state, by) = state_of.get(&(r.repository.clone(), r.suite.clone(), r.name.clone())).cloned().unwrap_or((FlakyState::Active, None));
        *counts.entry(state.as_str().to_string()).or_default() += 1;
        if !req.states.is_empty() && !req.states.contains(&state) {
            continue;
        }
        tests.push(FlakyTest {
            failure_rate: if r.runs > 0 { r.failures as f64 * 100.0 / r.runs as f64 } else { 0.0 },
            repository: r.repository,
            suite: r.suite,
            name: r.name,
            state,
            team: r.team,
            runs: r.runs,
            failures: r.failures,
            avg_ms: r.avg_ms,
            flaky_commits: r.flaky_commits,
            first_flake: r.first_flake,
            last_flake: r.last_flake,
            last_failed_trace_id: r.last_failed_trace,
            state_updated_by: by,
        });
    }
    Ok(Json(FlakyPage { tests, counts }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlakyStateUpdate {
    pub repository: String,
    pub suite: String,
    pub name: String,
    pub state: FlakyState,
}

#[utoipa::path(put, operation_id = "ci_set_flaky_state", path = "/api/v1/ci/flaky/state", request_body = FlakyStateUpdate, responses((status = 204)), tag = "ci")]
pub async fn set_flaky_state(State(st): State<AppState>, Writer(ctx): Writer, Json(u): Json<FlakyStateUpdate>) -> Result<axum::http::StatusCode, ApiError> {
    if [&u.repository, &u.suite, &u.name].iter().any(|s| s.is_empty() || s.len() > 500) {
        return Err(ApiError::BadRequest("repository, suite and name are required".into()));
    }
    sqlx::query(
        "INSERT INTO ci_test_states (org_id, repository, suite, name, state, updated_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) \
         ON CONFLICT (org_id, repository, suite, name) DO UPDATE SET state = EXCLUDED.state, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&u.repository)
    .bind(&u.suite)
    .bind(&u.name)
    .bind(u.state.as_str())
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "ci.flaky_test.state", &format!("{}/{}.{}", u.repository, u.suite, u.name), u.state.as_str()).await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ================================================================ DORA

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DoraRequest {
    pub range: TimeRange,
    /// Deployment environment (default production).
    pub env: Option<String>,
    /// `deployment.service.name` values; empty = all.
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DoraSummary {
    pub deployments: u64,
    pub failed_deployments: u64,
    pub deployments_per_day: f64,
    /// Median commit → production time.
    pub lead_time_ms: Option<f64>,
    /// Share of deployments that failed or caused an incident.
    pub change_failure_rate: Option<f64>,
    /// Median time to resolve incidents affecting these services.
    pub time_to_restore_ms: Option<f64>,
    /// elite | high | medium | low per metric (DORA research thresholds).
    pub ratings: BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DoraDay {
    pub day: i64,
    pub deployments: u64,
    pub failed: u64,
    pub lead_time_ms: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DoraService {
    pub service: String,
    pub repository: String,
    pub summary: DoraSummary,
    pub last_deploy: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DoraResponse {
    pub summary: DoraSummary,
    pub days: Vec<DoraDay>,
    pub services: Vec<DoraService>,
    pub environments: Vec<String>,
    pub incidents: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Deploy {
    service: String,
    repository: String,
    start: i64,
    end: i64,
    failed: bool,
    committed_at: i64,
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;

fn summarize(deploys: &[&Deploy], days: f64, incident_starts: &[(String, i64, Option<i64>)]) -> DoraSummary {
    let ok: Vec<&&Deploy> = deploys.iter().filter(|d| !d.failed).collect();
    let failed = deploys.len() - ok.len();
    let lead = median(ok.iter().filter(|d| d.committed_at > 0).map(|d| (d.end - d.committed_at) as f64).collect());
    // A successful deployment followed by an incident on the same service within a day also counts as a failed change.
    let caused_incident = ok
        .iter()
        .filter(|d| incident_starts.iter().any(|(svc, at, _)| svc == &d.service && *at >= d.end && (*at - d.end) as f64 <= DAY))
        .count();
    let cfr = (!deploys.is_empty()).then(|| (failed + caused_incident) as f64 * 100.0 / deploys.len() as f64);
    let services: Vec<&str> = deploys.iter().map(|d| d.service.as_str()).collect();
    let restore = median(
        incident_starts
            .iter()
            .filter(|(svc, ..)| services.contains(&svc.as_str()))
            .filter_map(|(_, s, r)| r.map(|r| (r - s) as f64))
            .collect(),
    );
    let per_day = ok.len() as f64 / days.max(1.0);
    let mut ratings = BTreeMap::new();
    ratings.insert("deploymentFrequency".into(), if per_day >= 1.0 { "elite" } else if per_day >= 1.0 / 7.0 { "high" } else if per_day >= 1.0 / 30.0 { "medium" } else { "low" }.to_string());
    if let Some(l) = lead {
        ratings.insert("leadTime".into(), if l < DAY { "elite" } else if l < 7.0 * DAY { "high" } else if l < 30.0 * DAY { "medium" } else { "low" }.to_string());
    }
    if let Some(c) = cfr {
        ratings.insert("changeFailureRate".into(), if c <= 5.0 { "elite" } else if c <= 10.0 { "high" } else if c <= 15.0 { "medium" } else { "low" }.to_string());
    }
    if let Some(r) = restore {
        ratings.insert("timeToRestore".into(), if r < HOUR { "elite" } else if r < DAY { "high" } else if r < 7.0 * DAY { "medium" } else { "low" }.to_string());
    }
    DoraSummary {
        deployments: deploys.len() as u64,
        failed_deployments: failed as u64,
        deployments_per_day: (per_day * 100.0).round() / 100.0,
        lead_time_ms: lead,
        change_failure_rate: cfr.map(|c| (c * 10.0).round() / 10.0),
        time_to_restore_ms: restore,
        ratings,
    }
}

#[utoipa::path(post, operation_id = "ci_dora", path = "/api/v1/ci/dora", request_body = DoraRequest, responses((status = 200, body = DoraResponse)), tag = "ci")]
pub async fn dora(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<DoraRequest>) -> ApiResult<DoraResponse> {
    req.range.validate_loose()?;
    let env = req.env.clone().unwrap_or_else(|| "production".into());
    let sql = format!(
        "SELECT SpanAttributes['deployment.service.name'] AS service, {REPO} AS repository, toInt64(toUnixTimestamp64Milli(Timestamp)) AS start, \
         toInt64(toUnixTimestamp64Milli(Timestamp) + intDiv(Duration, 1000000)) AS end, {STATUS} = 'error' AS failed, \
         toInt64OrZero(ResourceAttributes['vcs.revision.committed_at']) AS committed_at \
         FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {JOB} != '' AND SpanAttributes['deployment.environment.name'] = ? \
           AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND (empty(?) OR has(?, service)) ORDER BY start"
    );
    let deploys = st.ch(&org).query(&sql).bind(&env).bind(req.range.from).bind(req.range.to).bind(&req.services).bind(&req.services).fetch_all::<Deploy>().await?;
    let environments: Vec<String> = st
        .ch(&org)
        .query(&format!("SELECT DISTINCT SpanAttributes['deployment.environment.name'] AS e FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND {JOB} != '' AND e != '' AND Timestamp > now() - INTERVAL 30 DAY ORDER BY e"))
        .fetch_all::<String>()
        .await?;
    // Incidents (declared in range) touching deployed services, for change failure and restore times.
    let incidents: Vec<(Vec<String>, i64, Option<i64>)> = sqlx::query_as(
        "SELECT services, (extract(epoch FROM declared_at) * 1000)::bigint, (extract(epoch FROM resolved_at) * 1000)::bigint FROM incidents \
         WHERE org_id = $1 AND NOT test AND declared_at >= to_timestamp($2) AND declared_at <= to_timestamp($3)",
    )
    .bind(&org)
    .bind(req.range.from as f64 / 1000.0)
    .bind(req.range.to as f64 / 1000.0)
    .fetch_all(&st.postgres)
    .await?;
    let incident_starts: Vec<(String, i64, Option<i64>)> = incidents.iter().flat_map(|(svcs, s, r)| svcs.iter().map(move |svc| (svc.clone(), *s, *r))).collect();
    let days = (req.range.to - req.range.from) as f64 / DAY;
    let all: Vec<&Deploy> = deploys.iter().collect();
    let summary = summarize(&all, days, &incident_starts);

    let mut per_day: BTreeMap<i64, (u64, u64, Vec<f64>)> = BTreeMap::new();
    let first_day = (req.range.from as f64 / DAY).floor() as i64;
    let last_day = (req.range.to as f64 / DAY).floor() as i64;
    for d in first_day..=last_day {
        per_day.insert(d * DAY as i64, (0, 0, Vec::new()));
    }
    for d in &deploys {
        let e = per_day.entry((d.start as f64 / DAY).floor() as i64 * DAY as i64).or_default();
        e.0 += 1;
        if d.failed {
            e.1 += 1;
        } else if d.committed_at > 0 {
            e.2.push((d.end - d.committed_at) as f64);
        }
    }
    let days_out = per_day.into_iter().map(|(day, (n, f, lead))| DoraDay { day, deployments: n, failed: f, lead_time_ms: median(lead) }).collect();

    let mut by_service: BTreeMap<String, Vec<&Deploy>> = BTreeMap::new();
    for d in &deploys {
        by_service.entry(d.service.clone()).or_default().push(d);
    }
    let mut services: Vec<DoraService> = by_service
        .into_iter()
        .map(|(service, ds)| DoraService {
            repository: ds[0].repository.clone(),
            last_deploy: ds.iter().map(|d| d.start).max().unwrap_or(0),
            summary: summarize(&ds, days, &incident_starts),
            service,
        })
        .collect();
    services.sort_by(|a, b| b.summary.deployments.cmp(&a.summary.deployments));
    let svc_names: Vec<&str> = deploys.iter().map(|d| d.service.as_str()).collect();
    let incident_count = incidents.iter().filter(|(svcs, ..)| svcs.iter().any(|s| svc_names.contains(&s.as_str()))).count() as u64;
    Ok(Json(DoraResponse { summary, days: days_out, services, environments, incidents: incident_count }))
}

// ================================================================ repositories

#[derive(Serialize, ToSchema, clickhouse::Row, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiRepository {
    pub repository: String,
    pub url: String,
    pub provider: String,
    pub pipelines: Vec<String>,
    pub runs: u64,
    pub failures: u64,
    pub last_run: i64,
    pub last_status: String,
}

/// Repositories that reported pipeline runs in the last 30 days.
#[utoipa::path(get, operation_id = "ci_repositories", path = "/api/v1/ci/repositories", responses((status = 200, body = Vec<CiRepository>)), tag = "ci")]
pub async fn repositories(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<CiRepository>> {
    let sql = format!(
        "SELECT {REPO} AS repository, any(ResourceAttributes['vcs.repository.url.full']) AS url, any({PROVIDER}) AS provider, groupUniqArray({PIPELINE}) AS pipelines, \
         count() AS runs, countIf({STATUS} = 'error') AS failures, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_run, argMax({STATUS}, Timestamp) AS last_status \
         FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND ParentSpanId = '' AND Timestamp > now() - INTERVAL 30 DAY GROUP BY repository ORDER BY repository"
    );
    Ok(Json(st.ch(&org).query(&sql).fetch_all::<CiRepository>().await?))
}
