//! AI Guard: runtime protection for LLM applications and agents. The tracers' AI Guard SDKs ask
//! `POST /api/v2/ai-guard/evaluate` (`datadog_api::ai_guard`) whether each step of a conversation
//! is safe; `ai_guard_engine` decides and every evaluation is stored in ClickHouse
//! `otel.ai_guard_evaluations` (written here: the server produces these rows itself).
//!
//! - Investigate: evaluations with filters (assessment, blocked, service, env, type, category,
//!   free text), the summary cards and charts, and one evaluation with the conversation it belongs
//!   to. The SDK's own `ai_guard` span (client IP, user, trace) is joined in when it reached APM.
//! - Discover: LLM spans (`llm_attrs`) by service and model provider, and which services call
//!   AI Guard.
//! - Signals: default `application_security` detection rules on the evaluations (`@ai_guard.*`
//!   queries), evaluated by `security_engine` through `group_counts` / `entities` below.
//! - Settings per org (Postgres `ai_guard_settings`), and the Playground (evaluate, don't store).

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::ai_guard_engine::{self as engine, AiGuardMessage, AiGuardOptions, AiGuardVerdict};
use super::llm_attrs as attrs;
use super::logs::TimeRange;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const UNSAFE: &str = "Action != 'ALLOW'";

// ================================================================ settings

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardSettings {
    /// Block unsafe steps of every service.
    pub blocking_enabled: bool,
    /// Services in blocking mode when `blocking_enabled` is off.
    #[serde(default)]
    pub blocking_services: Vec<String>,
    pub threshold: f64,
    pub disabled_categories: Vec<String>,
    pub sds_enabled: bool,
}

impl Default for AiGuardSettings {
    fn default() -> Self {
        AiGuardSettings { blocking_enabled: false, blocking_services: vec![], threshold: engine::DEFAULT_THRESHOLD, disabled_categories: vec![], sds_enabled: true }
    }
}

pub async fn load_settings(st: &AppState, org: &str) -> Result<AiGuardSettings, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        blocking_enabled: bool,
        blocking_services: Vec<String>,
        threshold: f64,
        disabled_categories: Vec<String>,
        sds_enabled: bool,
    }
    let row: Option<Row> =
        sqlx::query_as("SELECT blocking_enabled, blocking_services, threshold, disabled_categories, sds_enabled FROM ai_guard_settings WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row
        .map(|r| AiGuardSettings { blocking_enabled: r.blocking_enabled, blocking_services: r.blocking_services, threshold: r.threshold, disabled_categories: r.disabled_categories, sds_enabled: r.sds_enabled })
        .unwrap_or_default())
}

impl AiGuardSettings {
    pub fn blocks(&self, service: &str) -> bool {
        self.blocking_enabled || self.blocking_services.iter().any(|s| s == service)
    }
}

#[utoipa::path(get, path = "/api/v1/ai-guard/settings", operation_id = "ai_guard_settings", responses((status = 200, body = AiGuardSettings)), tag = "ai-guard")]
pub async fn settings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<AiGuardSettings> {
    Ok(Json(load_settings(&st, &ctx.org_id).await?))
}

#[utoipa::path(put, path = "/api/v1/ai-guard/settings", operation_id = "ai_guard_update_settings", request_body = AiGuardSettings, responses((status = 200, body = AiGuardSettings)), tag = "ai-guard")]
pub async fn update_settings(State(st): State<AppState>, Writer(ctx): Writer, Json(s): Json<AiGuardSettings>) -> ApiResult<AiGuardSettings> {
    if !(0.05..=0.99).contains(&s.threshold) {
        return Err(ApiError::BadRequest("threshold must be between 0.05 and 0.99".into()));
    }
    if let Some(c) = s.disabled_categories.iter().find(|c| !engine::CATEGORIES.iter().any(|(k, _)| k == c)) {
        return Err(ApiError::BadRequest(format!("unknown attack category {c}")));
    }
    sqlx::query(
        "INSERT INTO ai_guard_settings (org_id, blocking_enabled, blocking_services, threshold, disabled_categories, sds_enabled) VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (org_id) DO UPDATE SET blocking_enabled = $2, blocking_services = $3, threshold = $4, disabled_categories = $5, sds_enabled = $6, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(s.blocking_enabled)
    .bind(&s.blocking_services)
    .bind(s.threshold)
    .bind(&s.disabled_categories)
    .bind(s.sds_enabled)
    .execute(&st.postgres)
    .await?;
    Ok(Json(s))
}

// ================================================================ evaluate and record

/// The request's `meta` (service, env, tool policies, threshold override, SDS override).
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub service: String,
    pub env: String,
    pub threshold: Option<f64>,
    pub tool_policies: BTreeMap<String, String>,
    pub sds: Option<bool>,
}

impl Meta {
    pub fn from_json(v: &Value) -> Meta {
        let threshold = match v.get("confidence_threshold") {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::Object(o)) => o.get("flagging_global").and_then(Value::as_f64),
            _ => None,
        };
        Meta {
            service: v.get("service").and_then(Value::as_str).unwrap_or_default().to_string(),
            env: v.get("env").and_then(Value::as_str).unwrap_or_default().to_string(),
            threshold,
            tool_policies: v.get("tool_policies").and_then(Value::as_object).map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_uppercase()))).collect()).unwrap_or_default(),
            sds: v.get("is_sds_enabled_override").and_then(Value::as_bool),
        }
    }
}

/// Evaluates with the org's settings; returns the verdict and whether blocking is on.
pub async fn evaluate(st: &AppState, org: &str, messages: &[AiGuardMessage], meta: &Meta) -> Result<(AiGuardVerdict, AiGuardSettings), ApiError> {
    let s = load_settings(st, org).await?;
    let opts = AiGuardOptions { threshold: meta.threshold.unwrap_or(s.threshold).clamp(0.01, 0.999), tool_policies: meta.tool_policies.clone(), sds: meta.sds.unwrap_or(s.sds_enabled), disabled: s.disabled_categories.clone() };
    Ok((engine::evaluate(messages, &opts), s))
}

/// Steps of one run share the service and the opening user message.
pub fn conversation_key(service: &str, messages: &[AiGuardMessage]) -> String {
    let opening = messages.iter().find(|m| m.role == "user").map(AiGuardMessage::text).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(service.as_bytes());
    h.update([0]);
    h.update(opening.as_bytes());
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// What the row shows as content: the step's text, or its tool call.
fn content_of(messages: &[AiGuardMessage]) -> String {
    let Some(last) = messages.last() else { return String::new() };
    let s = if let Some(c) = last.tool_calls.first() { format!("{}({})", c.function.name, c.function.arguments) } else { last.text() };
    s.chars().take(4000).collect()
}

pub struct Record<'a> {
    pub id: &'a str,
    pub meta: &'a Meta,
    pub messages: &'a [AiGuardMessage],
    pub verdict: &'a AiGuardVerdict,
    pub blocking_enabled: bool,
    pub duration_ms: f64,
    pub source: &'a str,
    pub language: &'a str,
}

/// Stores one evaluation (`otel.ai_guard_evaluations`, OrgId from the client's org parameter).
pub async fn record(st: &AppState, org: &str, r: Record<'_>) -> Result<(), ApiError> {
    let step = engine::step(r.messages);
    let (names, probs): (Vec<String>, Vec<f64>) = r.verdict.tag_probs.iter().map(|(k, v)| (k.clone(), *v)).unzip();
    let sds_tags: Vec<String> = r.verdict.sds_findings.iter().map(|f| f.rule_tag.clone()).collect();
    let mut sds_cats: Vec<String> = r.verdict.sds_findings.iter().map(|f| f.category.clone()).collect();
    sds_cats.sort();
    sds_cats.dedup();
    let blocked = r.blocking_enabled && r.verdict.action != "ALLOW";
    st.ch(org)
        .query(
            "INSERT INTO otel.ai_guard_evaluations (OrgId, Timestamp, Id, Conversation, Service, Env, Target, Direction, ToolName, Action, Blocked, BlockingEnabled, Reason, Tags, TagNames, TagProbs, \
             SdsTags, SdsCategories, Sds, Content, Messages, DurationMs, Source, Language) \
             SELECT {oc_org:String}, now64(3), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
        )
        .bind(r.id)
        .bind(conversation_key(&r.meta.service, r.messages))
        .bind(&r.meta.service)
        .bind(&r.meta.env)
        .bind(step.target)
        .bind(step.direction)
        .bind(&step.tool_name)
        .bind(&r.verdict.action)
        .bind(blocked as u8)
        .bind(r.blocking_enabled as u8)
        .bind(&r.verdict.reason)
        .bind(&r.verdict.tags)
        .bind(names)
        .bind(probs)
        .bind(sds_tags)
        .bind(sds_cats)
        .bind(serde_json::to_string(&r.verdict.sds_findings).unwrap_or_default())
        .bind(content_of(r.messages))
        .bind(serde_json::to_string(r.messages).unwrap_or_default())
        .bind(r.duration_ms)
        .bind(r.source)
        .bind(r.language)
        .execute()
        .await?;
    Ok(())
}

// ================================================================ search syntax

/// `@ai_guard.*` keys (and `service`, `env`) → column expressions.
fn column(key: &str) -> Option<(&'static str, bool)> {
    // (expression with one ? for the value, array column)
    Some(match key {
        "service" => ("Service = ?", false),
        "env" => ("Env = ?", false),
        "@ai_guard.action" => ("Action = upper(?)", false),
        "@ai_guard.target" | "@ai_guard.type" => ("Target = lower(?)", false),
        "@ai_guard.direction" => ("Direction = lower(?)", false),
        "@ai_guard.tool_name" => ("ToolName = ?", false),
        "@ai_guard.source" => ("Source = ?", false),
        "@ai_guard.attack_categories" | "@ai_guard.tags" => ("has(Tags, ?)", true),
        "@ai_guard.sds.category" => ("has(SdsCategories, ?)", true),
        "@ai_guard.sds.rule_tag" => ("has(SdsTags, ?)", true),
        "@ai_guard.usr.id" => ("UserId = ?", false),
        "@ai_guard.http.client_ip" => ("ClientIp = ?", false),
        _ => return None,
    })
}

/// A query (`key:value`, `key:(a OR b)`, `-key:value`, free text) → SQL with `?` params.
pub fn query_sql(q: &str) -> Result<(String, Vec<String>), String> {
    let mut parts: Vec<String> = Vec::new();
    let mut params = Vec::new();
    let mut tokens = Vec::new();
    // Split on spaces outside parentheses and quotes.
    let (mut cur, mut depth, mut quoted) = (String::new(), 0, false);
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            ' ' if depth == 0 && !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    for t in tokens {
        if t == "AND" {
            continue;
        }
        let (neg, t) = t.strip_prefix('-').map_or((false, t.as_str()), |r| (true, r));
        let Some((key, value)) = t.split_once(':').filter(|(k, _)| !k.is_empty() && !k.starts_with('"')) else {
            let text = t.trim_matches('"');
            parts.push(format!("{}positionCaseInsensitive(Content, ?) > 0", if neg { "NOT " } else { "" }));
            params.push(text.to_string());
            continue;
        };
        let values: Vec<String> = value.trim_start_matches('(').trim_end_matches(')').split(" OR ").map(|v| v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty()).collect();
        let clause = match key {
            "@ai_guard.blocked" => match values.first().map(String::as_str) {
                Some("true") => "Blocked = 1".to_string(),
                Some("false") => "Blocked = 0".to_string(),
                _ => return Err(format!("{key} is true or false")),
            },
            "@ai_guard.assessment" => match values.first().map(|v| v.to_lowercase()).as_deref() {
                Some("unsafe") => UNSAFE.to_string(),
                Some("safe") => "Action = 'ALLOW'".to_string(),
                _ => return Err(format!("{key} is safe or unsafe")),
            },
            "@ai_guard.sds" => match values.first().map(String::as_str) {
                Some("true") => "notEmpty(SdsTags)".to_string(),
                Some("false") => "empty(SdsTags)".to_string(),
                _ => return Err(format!("{key} is true or false")),
            },
            _ => {
                let Some((expr, _)) = column(key) else { return Err(format!("unknown field {key}")) };
                let ors: Vec<&str> = values.iter().map(|_| expr).collect();
                params.extend(values.iter().cloned());
                format!("({})", ors.join(" OR "))
            }
        };
        parts.push(if neg { format!("NOT ({clause})") } else { clause });
    }
    Ok((if parts.is_empty() { "1".into() } else { parts.join(" AND ") }, params))
}

/// The filters of the Investigate page.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardQuery {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// safe | unsafe
    #[serde(default)]
    pub assessment: Option<String>,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
    /// prompt | tool
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
}

fn where_clause(q: &AiGuardQuery) -> Result<(String, Vec<String>), ApiError> {
    let (mut w, mut p) = query_sql(&q.query).map_err(ApiError::BadRequest)?;
    w = format!("Timestamp >= fromUnixTimestamp64Milli(toInt64(?)) AND Timestamp <= fromUnixTimestamp64Milli(toInt64(?)) AND ({w})");
    p.splice(0..0, [q.range.from.to_string(), q.range.to.to_string()]);
    match q.assessment.as_deref() {
        Some("unsafe") => w.push_str(&format!(" AND {UNSAFE}")),
        Some("safe") => w.push_str(" AND Action = 'ALLOW'"),
        _ => {}
    }
    if let Some(b) = q.blocked {
        w.push_str(if b { " AND Blocked = 1" } else { " AND Blocked = 0" });
    }
    for (v, expr) in [(&q.service, "Service = ?"), (&q.env, "Env = ?"), (&q.target, "Target = ?"), (&q.category, "has(Tags, ?)")] {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            w.push_str(&format!(" AND {expr}"));
            p.push(v.clone());
        }
    }
    Ok((w, p))
}

fn bind_all(mut q: clickhouse::query::Query, params: &[String]) -> clickhouse::query::Query {
    for p in params {
        q = q.bind(p);
    }
    q
}

// ================================================================ Investigate

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardSeriesPoint {
    pub t: i64,
    pub safe: u64,
    pub unsafe_count: u64,
    pub blocked: u64,
    pub avg: f64,
    pub p75: f64,
    pub p95: f64,
    pub p99: f64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardSummary {
    pub total: u64,
    pub unsafe_count: u64,
    pub unsafe_not_blocked: u64,
    pub blocked: u64,
    pub sensitive: u64,
    pub step_ms: i64,
    pub series: Vec<AiGuardSeriesPoint>,
    /// Values for the filter dropdowns.
    pub services: Vec<String>,
    pub envs: Vec<String>,
    pub categories: Vec<String>,
    pub blocking_enabled: bool,
}

fn step_ms(range: &TimeRange) -> i64 {
    let span = (range.to - range.from).max(60_000);
    let raw = span / 60;
    [60_000, 300_000, 900_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000, 86_400_000].into_iter().find(|s| *s >= raw).unwrap_or(86_400_000)
}

#[utoipa::path(post, path = "/api/v1/ai-guard/summary", operation_id = "ai_guard_summary", request_body = AiGuardQuery, responses((status = 200, body = AiGuardSummary)), tag = "ai-guard")]
pub async fn summary(State(st): State<AppState>, ctx: AuthContext, Json(q): Json<AiGuardQuery>) -> ApiResult<AiGuardSummary> {
    let org = &ctx.org_id;
    let (w, p) = where_clause(&q)?;
    let step = step_ms(&q.range);
    #[derive(clickhouse::Row, Deserialize)]
    struct T {
        total: u64,
        bad: u64,
        open: u64,
        blocked: u64,
        sensitive: u64,
    }
    let totals = bind_all(
        st.ch(org).query(&format!(
            "SELECT count() AS total, countIf({UNSAFE}) AS bad, countIf({UNSAFE} AND Blocked = 0) AS open, countIf(Blocked = 1) AS blocked, countIf(notEmpty(SdsTags)) AS sensitive \
             FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND {w}"
        )),
        &p,
    );
    #[derive(clickhouse::Row, Deserialize)]
    struct B {
        t: i64,
        safe: u64,
        bad: u64,
        blocked: u64,
        avg: f64,
        q: Vec<f64>,
    }
    let series = bind_all(
        st.ch(org).query(&format!(
            "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, countIf(Action = 'ALLOW') AS safe, countIf({UNSAFE} AND Blocked = 0) AS bad, countIf(Blocked = 1) AS blocked, \
             avg(DurationMs) AS avg, quantiles(0.75, 0.95, 0.99)(DurationMs) AS q FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY t ORDER BY t"
        )),
        &p,
    );
    #[derive(clickhouse::Row, Deserialize)]
    struct V {
        services: Vec<String>,
        envs: Vec<String>,
        categories: Vec<String>,
    }
    let values = st
        .ch(org)
        .query(
            "SELECT groupUniqArray(100)(Service) AS services, groupUniqArray(20)(Env) AS envs, groupUniqArrayArray(50)(Tags) AS categories FROM otel.ai_guard_evaluations \
             WHERE OrgId = {oc_org:String} AND Timestamp >= now64(3) - INTERVAL 30 DAY",
        );
    let (t, buckets, v) = tokio::try_join!(totals.fetch_one::<T>(), series.fetch_all::<B>(), values.fetch_one::<V>())?;
    let s = load_settings(&st, org).await?;
    let mut services = v.services;
    services.sort();
    let mut envs = v.envs;
    envs.sort();
    let mut categories = v.categories;
    categories.sort_by_key(|c| engine::CATEGORIES.iter().position(|(k, _)| k == c).unwrap_or(99));
    let by_t: HashMap<i64, B> = buckets.into_iter().map(|b| (b.t, b)).collect();
    let r = |x: f64| (x * 100.0).round() / 100.0;
    let mut out = Vec::new();
    let mut ts = q.range.from / step * step;
    while ts <= q.range.to && out.len() < 400 {
        out.push(match by_t.get(&ts) {
            Some(b) => AiGuardSeriesPoint { t: ts, safe: b.safe, unsafe_count: b.bad, blocked: b.blocked, avg: r(b.avg), p75: r(b.q[0]), p95: r(b.q[1]), p99: r(b.q[2]) },
            None => AiGuardSeriesPoint { t: ts, safe: 0, unsafe_count: 0, blocked: 0, avg: 0.0, p75: 0.0, p95: 0.0, p99: 0.0 },
        });
        ts += step;
    }
    Ok(Json(AiGuardSummary {
        total: t.total,
        unsafe_count: t.bad,
        unsafe_not_blocked: t.open,
        blocked: t.blocked,
        sensitive: t.sensitive,
        step_ms: step,
        series: out,
        services,
        envs,
        categories,
        blocking_enabled: s.blocking_enabled,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema, clickhouse::Row)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardEvaluationRow {
    pub id: String,
    pub timestamp: i64,
    pub service: String,
    pub env: String,
    /// prompt | tool
    pub target: String,
    /// input | output
    pub direction: String,
    pub tool_name: String,
    /// ALLOW | DENY | ABORT
    pub action: String,
    pub blocked: u8,
    pub tags: Vec<String>,
    pub sds_categories: Vec<String>,
    pub sds_tags: Vec<String>,
    pub content: String,
    pub duration_ms: f64,
    pub conversation: String,
}

const ROW_COLS: &str = "Id AS id, toInt64(toUnixTimestamp64Milli(Timestamp)) AS timestamp, Service AS service, Env AS env, Target AS target, Direction AS direction, ToolName AS tool_name, \
     Action AS action, Blocked AS blocked, Tags AS tags, SdsCategories AS sds_categories, SdsTags AS sds_tags, substring(Content, 1, 400) AS content, DurationMs AS duration_ms, Conversation AS conversation";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardSearchRequest {
    #[serde(flatten)]
    pub q: AiGuardQuery,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub offset: Option<u64>,
    /// Oldest first.
    #[serde(default)]
    pub ascending: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AiGuardEvaluationPage {
    pub total: u64,
    pub rows: Vec<AiGuardEvaluationRow>,
}

#[utoipa::path(post, path = "/api/v1/ai-guard/evaluations", operation_id = "ai_guard_evaluations", request_body = AiGuardSearchRequest, responses((status = 200, body = AiGuardEvaluationPage)), tag = "ai-guard")]
pub async fn evaluations(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AiGuardSearchRequest>) -> ApiResult<AiGuardEvaluationPage> {
    let org = &ctx.org_id;
    let (w, p) = where_clause(&req.q)?;
    let (limit, offset) = (req.limit.unwrap_or(100).min(1000), req.offset.unwrap_or(0).min(100_000));
    let order = if req.ascending { "ASC" } else { "DESC" };
    let rows = bind_all(st.ch(org).query(&format!("SELECT {ROW_COLS} FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND {w} ORDER BY Timestamp {order} LIMIT {limit} OFFSET {offset}")), &p);
    let total = bind_all(st.ch(org).query(&format!("SELECT count() FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND {w}")), &p);
    let (rows, total) = tokio::try_join!(rows.fetch_all::<AiGuardEvaluationRow>(), total.fetch_one::<u64>())?;
    Ok(Json(AiGuardEvaluationPage { total, rows }))
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardCategoryScore {
    pub category: String,
    pub confidence: f64,
    /// At or above the threshold: reported as an attack category.
    pub flagged: bool,
    pub description: String,
}

/// One evaluated step of the conversation timeline.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardStep {
    pub id: String,
    pub timestamp: i64,
    pub target: String,
    pub direction: String,
    pub tool_name: String,
    pub action: String,
    pub blocked: bool,
    pub tags: Vec<String>,
    /// The step's message (the last one of its request).
    #[schema(value_type = Object)]
    pub message: Value,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardEvaluationDetail {
    #[serde(flatten)]
    pub row: AiGuardEvaluationRow,
    pub reason: String,
    pub blocking_enabled: bool,
    pub source: String,
    pub language: String,
    pub threshold: f64,
    pub scores: Vec<AiGuardCategoryScore>,
    #[schema(value_type = Vec<Object>)]
    pub sds_findings: Value,
    /// The request's messages, as sent.
    #[schema(value_type = Vec<Object>)]
    pub messages: Value,
    /// Messages before the first evaluated step of the run.
    #[schema(value_type = Vec<Object>)]
    pub context: Value,
    /// Every evaluated step of the run, oldest first (this one included).
    pub steps: Vec<AiGuardStep>,
    /// From the SDK's `ai_guard` span, when it reached APM.
    pub client_ip: String,
    pub user_id: String,
    pub user_email: String,
    pub trace_id: String,
    pub span_id: String,
}

#[derive(clickhouse::Row, Deserialize)]
struct Full {
    #[serde(flatten)]
    row: AiGuardEvaluationRow,
    reason: String,
    blocking_enabled: u8,
    source: String,
    language: String,
    tag_names: Vec<String>,
    tag_probs: Vec<f64>,
    sds: String,
    messages: String,
}

#[utoipa::path(get, path = "/api/v1/ai-guard/evaluations/{id}", operation_id = "ai_guard_evaluation", params(("id" = String, Path)), responses((status = 200, body = AiGuardEvaluationDetail), (status = 404)), tag = "ai-guard")]
pub async fn evaluation(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<AiGuardEvaluationDetail> {
    let org = &ctx.org_id;
    let full: Option<Full> = st
        .ch(org)
        .query(&format!(
            "SELECT {ROW_COLS}, Reason AS reason, BlockingEnabled AS blocking_enabled, Source AS source, Language AS language, TagNames AS tag_names, TagProbs AS tag_probs, Sds AS sds, \
             Messages AS messages FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND Id = ? LIMIT 1"
        ))
        .bind(&id)
        .fetch_optional()
        .await?;
    let Some(f) = full else { return Err(ApiError::NotFound(format!("no evaluation {id}"))) };
    let settings = load_settings(&st, org).await?;
    let messages: Value = serde_json::from_str(&f.messages).unwrap_or(json!([]));
    // The run: steps of the same conversation within an hour.
    #[derive(clickhouse::Row, Deserialize)]
    struct S {
        id: String,
        timestamp: i64,
        target: String,
        direction: String,
        tool_name: String,
        action: String,
        blocked: u8,
        tags: Vec<String>,
        messages: String,
    }
    let around = f.row.timestamp;
    let steps: Vec<S> = st
        .ch(org)
        .query(
            "SELECT Id AS id, toInt64(toUnixTimestamp64Milli(Timestamp)) AS timestamp, Target AS target, Direction AS direction, ToolName AS tool_name, Action AS action, Blocked AS blocked, \
             Tags AS tags, Messages AS messages FROM otel.ai_guard_evaluations WHERE OrgId = {oc_org:String} AND Conversation = ? AND Service = ? \
             AND Timestamp >= fromUnixTimestamp64Milli(toInt64(?)) AND Timestamp <= fromUnixTimestamp64Milli(toInt64(?)) ORDER BY Timestamp LIMIT 50",
        )
        .bind(&f.row.conversation)
        .bind(&f.row.service)
        .bind(around - 3_600_000)
        .bind(around + 3_600_000)
        .fetch_all()
        .await?;
    let mut context = json!([]);
    let steps: Vec<AiGuardStep> = steps
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let msgs: Vec<Value> = serde_json::from_str(&s.messages).unwrap_or_default();
            if i == 0 {
                context = json!(msgs[..msgs.len().saturating_sub(1)]);
            }
            AiGuardStep { id: s.id, timestamp: s.timestamp, target: s.target, direction: s.direction, tool_name: s.tool_name, action: s.action, blocked: s.blocked == 1, tags: s.tags, message: msgs.last().cloned().unwrap_or(Value::Null) }
        })
        .collect();
    let mut scores: Vec<AiGuardCategoryScore> = f
        .tag_names
        .iter()
        .zip(&f.tag_probs)
        .map(|(c, p)| AiGuardCategoryScore { category: c.clone(), confidence: *p, flagged: f.row.tags.contains(c), description: engine::describe(c).to_string() })
        .collect();
    scores.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    // The SDK's span: closest `ai_guard` span of the service around the evaluation.
    #[derive(clickhouse::Row, Deserialize)]
    struct Sp {
        trace_id: String,
        span_id: String,
        ip: String,
        usr: String,
        email: String,
    }
    let span: Option<Sp> = st
        .ch(org)
        .query(
            "SELECT TraceId AS trace_id, SpanId AS span_id, SpanAttributes['ai_guard.http.client_ip'] AS ip, SpanAttributes['ai_guard.usr.id'] AS usr, SpanAttributes['usr.email'] AS email \
             FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND ServiceName = ? AND SpanAttributes['ai_guard.action'] != '' AND SpanAttributes['ai_guard.target'] = ? \
             AND Timestamp >= fromUnixTimestamp64Milli(toInt64(?)) AND Timestamp <= fromUnixTimestamp64Milli(toInt64(?)) \
             ORDER BY abs(toInt64(toUnixTimestamp64Milli(Timestamp)) - toInt64(?)) LIMIT 1",
        )
        .bind(&f.row.service)
        .bind(&f.row.target)
        .bind(around - 5_000)
        .bind(around + 5_000)
        .bind(around)
        .fetch_optional()
        .await
        .ok()
        .flatten();
    let span = span.unwrap_or(Sp { trace_id: String::new(), span_id: String::new(), ip: String::new(), usr: String::new(), email: String::new() });
    Ok(Json(AiGuardEvaluationDetail {
        reason: f.reason,
        blocking_enabled: f.blocking_enabled == 1,
        source: f.source,
        language: f.language,
        threshold: settings.threshold,
        scores,
        sds_findings: serde_json::from_str(&f.sds).unwrap_or(json!([])),
        messages,
        context,
        steps,
        client_ip: span.ip,
        user_id: span.usr,
        user_email: span.email,
        trace_id: span.trace_id,
        span_id: span.span_id,
        row: f.row,
    }))
}

// ================================================================ Playground

#[derive(Debug, Deserialize, ToSchema)]
pub struct AiGuardPlaygroundRequest {
    #[schema(value_type = Vec<Object>)]
    pub messages: Value,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub meta: Value,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiGuardPlaygroundResult {
    pub action: String,
    pub reason: String,
    pub target: String,
    pub direction: String,
    pub tool_name: String,
    pub blocking_enabled: bool,
    pub threshold: f64,
    pub scores: Vec<AiGuardCategoryScore>,
    pub sds_findings: Vec<engine::AiGuardSdsFinding>,
    pub matches: Vec<engine::AiGuardMatch>,
}

#[utoipa::path(post, path = "/api/v1/ai-guard/playground", operation_id = "ai_guard_playground", request_body = AiGuardPlaygroundRequest, responses((status = 200, body = AiGuardPlaygroundResult)), tag = "ai-guard")]
pub async fn playground(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AiGuardPlaygroundRequest>) -> ApiResult<AiGuardPlaygroundResult> {
    let messages = engine::parse_messages(&req.messages).map_err(ApiError::BadRequest)?;
    let meta = Meta::from_json(&req.meta);
    let (v, s) = evaluate(&st, &ctx.org_id, &messages, &meta).await?;
    let threshold = meta.threshold.unwrap_or(s.threshold);
    let step = engine::step(&messages);
    let mut scores: Vec<AiGuardCategoryScore> =
        v.tag_probs.iter().map(|(c, p)| AiGuardCategoryScore { category: c.clone(), confidence: *p, flagged: v.tags.contains(c), description: engine::describe(c).to_string() }).collect();
    scores.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    Ok(Json(AiGuardPlaygroundResult {
        action: v.action,
        reason: v.reason,
        target: step.target.into(),
        direction: step.direction.into(),
        tool_name: step.tool_name,
        blocking_enabled: s.blocks(&meta.service),
        threshold,
        scores,
        sds_findings: v.sds_findings,
        matches: v.matches,
    }))
}

// ================================================================ Discover

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiProvider {
    /// Normalized provider key (openai, anthropic, google, deepseek, azureopenai, bedrock, vertexai, mistral, cohere, meta, …).
    pub provider: String,
    pub tokens: f64,
    pub requests: u64,
    pub services: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiService {
    pub service: String,
    pub tokens: f64,
    pub requests: u64,
    pub providers: Vec<String>,
    pub models: Vec<String>,
    /// Calls AI Guard (evaluations in the range).
    pub protected: bool,
    pub evaluations: u64,
    pub unsafe_count: u64,
    pub sensitive: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AiDiscover {
    pub providers: Vec<AiProvider>,
    pub services: Vec<AiService>,
}

/// Raw `gen_ai.system` / provider names → one key per provider.
pub fn provider_key(raw: &str) -> String {
    let r = raw.to_lowercase();
    let k = match () {
        _ if r.contains("azure") || r.starts_with("az.") => "azureopenai",
        _ if r.contains("bedrock") || r.contains("aws") => "bedrock",
        _ if r.contains("vertex") => "vertexai",
        _ if r.contains("openai") => "openai",
        _ if r.contains("anthropic") || r.contains("claude") => "anthropic",
        _ if r.contains("gemini") || r.contains("google") || r == "gcp.gen_ai" => "google",
        _ if r.contains("deepseek") => "deepseek",
        _ if r.contains("mistral") => "mistral",
        _ if r.contains("cohere") => "cohere",
        _ if r.contains("meta") || r.contains("llama") => "meta",
        _ if r.contains("hugging") => "huggingface",
        _ if r.contains("nvidia") => "nvidia",
        _ if r.contains("perplexity") => "perplexity",
        _ => return r,
    };
    k.to_string()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AiDiscoverRequest {
    pub range: TimeRange,
}

#[utoipa::path(post, path = "/api/v1/ai-guard/discover", operation_id = "ai_guard_discover", request_body = AiDiscoverRequest, responses((status = 200, body = AiDiscover)), tag = "ai-guard")]
pub async fn discover(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AiDiscoverRequest>) -> ApiResult<AiDiscover> {
    let org = &ctx.org_id;
    let (provider, model, call) = (attrs::first_sql(attrs::PROVIDER_KEYS), attrs::first_sql(attrs::MODEL_KEYS), attrs::is_call_sql());
    let (tin, tout) = (attrs::input_tokens_sql(), attrs::output_tokens_sql());
    #[derive(clickhouse::Row, Deserialize)]
    struct U {
        service: String,
        provider: String,
        models: Vec<String>,
        requests: u64,
        tokens: f64,
    }
    let usage = st
        .ch(org)
        .query(&format!(
            "SELECT ServiceName AS service, {provider} AS provider, groupUniqArray(10)({model}) AS models, count() AS requests, sum({tin}) + sum({tout}) AS tokens FROM otel.otel_traces \
             WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(toInt64(?)) AND Timestamp <= fromUnixTimestamp64Milli(toInt64(?)) AND {call} \
             GROUP BY service, provider ORDER BY tokens DESC LIMIT 2000"
        ))
        .bind(req.range.from)
        .bind(req.range.to);
    #[derive(clickhouse::Row, Deserialize)]
    struct G {
        service: String,
        n: u64,
        bad: u64,
        sensitive: u64,
    }
    let guarded = st
        .ch(org)
        .query(&format!(
            "SELECT Service AS service, count() AS n, countIf({UNSAFE}) AS bad, countIf(notEmpty(SdsTags)) AS sensitive FROM otel.ai_guard_evaluations \
             WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(toInt64(?)) AND Timestamp <= fromUnixTimestamp64Milli(toInt64(?)) GROUP BY service"
        ))
        .bind(req.range.from)
        .bind(req.range.to);
    let (usage, guarded) = tokio::try_join!(usage.fetch_all::<U>(), guarded.fetch_all::<G>())?;
    let guarded: HashMap<String, G> = guarded.into_iter().map(|g| (g.service.clone(), g)).collect();
    let mut providers: BTreeMap<String, AiProvider> = BTreeMap::new();
    let mut services: BTreeMap<String, AiService> = BTreeMap::new();
    for u in usage {
        let key = if u.provider.is_empty() { "other".to_string() } else { provider_key(&u.provider) };
        let p = providers.entry(key.clone()).or_insert_with(|| AiProvider { provider: key.clone(), tokens: 0.0, requests: 0, services: vec![] });
        p.tokens += u.tokens;
        p.requests += u.requests;
        if !p.services.contains(&u.service) {
            p.services.push(u.service.clone());
        }
        let g = guarded.get(&u.service);
        let s = services.entry(u.service.clone()).or_insert_with(|| AiService {
            service: u.service.clone(),
            tokens: 0.0,
            requests: 0,
            providers: vec![],
            models: vec![],
            protected: g.is_some(),
            evaluations: g.map_or(0, |g| g.n),
            unsafe_count: g.map_or(0, |g| g.bad),
            sensitive: g.map_or(0, |g| g.sensitive),
        });
        s.tokens += u.tokens;
        s.requests += u.requests;
        if !s.providers.contains(&key) {
            s.providers.push(key);
        }
        for m in u.models.into_iter().filter(|m| !m.is_empty()) {
            if !s.models.contains(&m) {
                s.models.push(m);
            }
        }
    }
    // Services that call AI Guard without LLM spans in APM.
    for (name, g) in guarded {
        services.entry(name.clone()).or_insert(AiService { service: name, tokens: 0.0, requests: 0, providers: vec![], models: vec![], protected: true, evaluations: g.n, unsafe_count: g.bad, sensitive: g.sensitive });
    }
    let mut providers: Vec<AiProvider> = providers.into_values().collect();
    providers.sort_by(|a, b| b.tokens.total_cmp(&a.tokens));
    let mut services: Vec<AiService> = services.into_values().collect();
    services.sort_by(|a, b| b.tokens.total_cmp(&a.tokens));
    Ok(Json(AiDiscover { providers, services }))
}

// ================================================================ detection rules

/// Default detection rules on the evaluations: (id, name, message, query, group by, cases, window, tags).
pub type AiGuardRule = (&'static str, &'static str, &'static str, &'static str, &'static [&'static str], &'static [(&'static str, &'static str, &'static str)], i64, &'static [&'static str]);

pub const DEFAULT_RULES: [AiGuardRule; 4] = [
    (
        "def-aig-001",
        "Unblocked & Unsafe Indirect prompt injection on AI service",
        "Content an agent fetched carried instructions to the model, and AI Guard flagged it without blocking. Look at the tool that returned it and turn on blocking for the service.",
        "@ai_guard.attack_categories:indirect-prompt-injection @ai_guard.blocked:false",
        &["service"],
        &[("a > 0", "high", "")],
        300,
        &["category:exploit", "tactic:ta0001-initial-access", "technique:t1190-exploit-public-facing-application"],
    ),
    (
        "def-aig-002",
        "Data exfiltrated from AI service",
        "A step of an AI service tried to move data outside the application and wasn't blocked. Check the destination and the data in the conversation.",
        "@ai_guard.attack_categories:data-exfiltration @ai_guard.blocked:false",
        &["service"],
        &[("a > 0", "critical", "")],
        300,
        &["category:exploit", "tactic:ta0010-exfiltration", "technique:t1567-exfiltration-over-web-service"],
    ),
    (
        "def-aig-003",
        "PII or credentials exfiltrated from AI service",
        "Model answers or tool calls of an AI service carried personal data or credentials out of the application.",
        "@ai_guard.direction:output @ai_guard.sds.category:(pii OR credentials)",
        &["service"],
        &[("a > 5", "high", "")],
        900,
        &["category:exploit", "tactic:ta0010-exfiltration", "technique:t1567-exfiltration-over-web-service"],
    ),
    (
        "def-aig-004",
        "Unblocked & Unsafe Traffic on AI service",
        "AI Guard flagged a burst of unsafe steps on an AI service that doesn't block them.",
        "@ai_guard.assessment:unsafe @ai_guard.blocked:false",
        &["service"],
        &[("a > 50", "high", "Condition 2"), ("a > 10", "medium", "Condition 1")],
        900,
        &["category:custom"],
    ),
];

/// The query is on AI Guard evaluations rather than on security traces.
pub fn is_ai_guard_query(q: &str) -> bool {
    q.contains("@ai_guard.")
}

fn group_col(k: &str) -> &'static str {
    match k {
        "service" => "Service",
        "env" => "Env",
        "@ai_guard.tool_name" => "ToolName",
        "@ai_guard.usr.id" => "UserId",
        "@ai_guard.http.client_ip" => "ClientIp",
        _ => "''",
    }
}

fn range_where(query: &str, range: TimeRange) -> Result<(String, Vec<String>), ApiError> {
    where_clause(&AiGuardQuery { range, query: query.to_string(), assessment: None, blocked: None, service: None, env: None, target: None, category: None })
}

/// Counts per group of the evaluations matching `query` in `range` (the security engine's threshold rules).
pub async fn group_counts(st: &AppState, org: &str, query: &str, group_by: &[String], range: TimeRange) -> Result<Vec<(Vec<String>, f64)>, ApiError> {
    let (w, p) = range_where(query, range)?;
    let g = if group_by.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", group_by.iter().map(|k| group_col(k)).collect::<Vec<_>>().join(", ")) };
    #[derive(clickhouse::Row, Deserialize)]
    struct R {
        g: Vec<String>,
        n: u64,
    }
    let rows: Vec<R> = bind_all(st.ch(org).query(&format!("SELECT {g} AS g, count() AS n FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY g ORDER BY n DESC LIMIT 1000")), &p).fetch_all().await?;
    Ok(rows.into_iter().map(|r| (r.g, r.n as f64)).collect())
}

/// The matching evaluations' count, top IPs, users, services and categories, as signal entities.
pub async fn entities(st: &AppState, org: &str, query: &str, group: &BTreeMap<String, String>, range: TimeRange) -> Result<(u64, Value, i64), ApiError> {
    let (mut w, mut p) = range_where(query, range)?;
    for (k, v) in group {
        let col = group_col(k);
        if col != "''" {
            w.push_str(&format!(" AND {col} = ?"));
            p.push(v.clone());
        }
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct E {
        n: u64,
        ips: Vec<String>,
        ip_count: u64,
        users: Vec<String>,
        user_count: u64,
        services: Vec<String>,
        categories: Vec<String>,
        tools: Vec<String>,
        first: i64,
    }
    let e: E = bind_all(
        st.ch(org).query(&format!(
            "SELECT count() AS n, topKIf(5)(ClientIp, ClientIp != '') AS ips, toUInt64(uniqExactIf(ClientIp, ClientIp != '')) AS ip_count, topKIf(5)(UserId, UserId != '') AS users, \
             toUInt64(uniqExactIf(UserId, UserId != '')) AS user_count, topK(3)(Service) AS services, topKArray(5)(Tags) AS categories, topKIf(5)(ToolName, ToolName != '') AS tools, \
             toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first FROM otel.ai_guard_evaluations WHERE OrgId = {{oc_org:String}} AND {w}"
        )),
        &p,
    )
    .fetch_one()
    .await?;
    let mut out = Map::new();
    for (k, top, count) in [("ip", e.ips, e.ip_count), ("user", e.users, e.user_count), ("service", e.services.clone(), e.services.len() as u64)] {
        if count > 0 {
            out.insert(k.into(), json!({ "top": top, "count": count }));
        }
    }
    if !e.categories.is_empty() {
        out.insert("attack_category".into(), json!({ "top": e.categories, "count": e.categories.len() }));
    }
    if !e.tools.is_empty() {
        out.insert("tool".into(), json!({ "top": e.tools, "count": e.tools.len() }));
    }
    Ok((e.n, Value::Object(out), e.first))
}

// ================================================================ demo

/// The demo org's settings: one service blocks, the others monitor, so the demo shows both.
pub async fn seed_demo(st: &AppState, org: &str) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO ai_guard_settings (org_id, blocking_services) VALUES ($1, ARRAY['support-agent']) ON CONFLICT (org_id) DO NOTHING").bind(org).execute(&st.postgres).await?;
    Ok(())
}

/// Category list for the UI.
#[derive(Debug, Serialize, ToSchema)]
pub struct AiGuardCategory {
    pub category: String,
    pub description: String,
}

#[utoipa::path(get, path = "/api/v1/ai-guard/categories", operation_id = "ai_guard_categories", responses((status = 200, body = Vec<AiGuardCategory>)), tag = "ai-guard")]
pub async fn categories() -> ApiResult<Vec<AiGuardCategory>> {
    Ok(Json(engine::CATEGORIES.iter().map(|(c, d)| AiGuardCategory { category: c.to_string(), description: d.to_string() }).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_become_bound_sql() {
        let (w, p) = query_sql("service:shopping-assistant @ai_guard.attack_categories:(jailbreak OR obfuscation) -@ai_guard.blocked:true refund").unwrap();
        assert_eq!(w, "(Service = ?) AND (has(Tags, ?) OR has(Tags, ?)) AND NOT (Blocked = 1) AND positionCaseInsensitive(Content, ?) > 0");
        assert_eq!(p, vec!["shopping-assistant", "jailbreak", "obfuscation", "refund"]);
        assert!(query_sql("@ai_guard.nope:x").is_err());
        assert_eq!(query_sql("").unwrap().0, "1");
        for (_, _, _, q, ..) in DEFAULT_RULES {
            assert!(query_sql(q).is_ok() && is_ai_guard_query(q), "{q}");
        }
    }

    #[test]
    fn providers_are_normalized() {
        assert_eq!(provider_key("openai"), "openai");
        assert_eq!(provider_key("az.ai.openai"), "azureopenai");
        assert_eq!(provider_key("aws.bedrock"), "bedrock");
        assert_eq!(provider_key("gcp.vertex_ai"), "vertexai");
        assert_eq!(provider_key("gemini"), "google");
    }

    #[test]
    fn steps_of_one_run_share_a_conversation() {
        let a = engine::parse_messages(&json!([{ "role": "user", "content": "track my order" }])).unwrap();
        let b = engine::parse_messages(&json!([{ "role": "user", "content": "track my order" }, { "role": "assistant", "content": "Sure" }])).unwrap();
        assert_eq!(conversation_key("support-agent", &a), conversation_key("support-agent", &b));
        assert_ne!(conversation_key("support-agent", &a), conversation_key("shopping-assistant", &a));
    }
}
