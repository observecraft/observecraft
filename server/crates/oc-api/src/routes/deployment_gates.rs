//! Software Delivery → Deployment Gates: rules a deployment must pass, evaluated on request by a
//! CI/CD pipeline (`POST /api/v1/deployment-gates/evaluate` with an application key; the org is the
//! key's). A gate is identified by service, environment and an identifier; its rules are
//! - **monitor**: fails when a monitor matching the rule's query (`service:billing env:prod`, tags
//!   and name words) is in ALERT or NO DATA at evaluation time;
//! - **faulty deployment detection**: fails when the version being deployed (`service.version` of
//!   its entry spans) has an error rate more than the allowed number of percentage points above the
//!   previous version's, over the last N minutes.
//!
//! A rule or a whole gate in dry run never blocks: the evaluation is recorded as `dry_run` when it
//! would have failed. Evaluations are synchronous and stored with each rule's outcome.

use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::apm::ENTRY;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_GATES: i64 = 500;
const MAX_RULES: usize = 20;
const MAX_FIELD_CHARS: usize = 200;
const MAX_QUERY_CHARS: usize = 1_000;
/// How far back the previous version is looked for when it no longer serves traffic.
const LOOKBACK_MS: i64 = 24 * 3_600_000;
const MAX_EVALUATIONS: i64 = 1_000;
/// Evaluations older than this are removed.
const RETENTION_DAYS: u32 = 90;

// ================================================================ model

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateRuleKind {
    Monitor,
    FaultyDeployment,
}

/// One rule of a gate. `query` is used by monitor rules; the window and thresholds by faulty
/// deployment detection.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GateRule {
    #[serde(rename = "type")]
    pub kind: GateRuleKind,
    #[serde(default)]
    pub name: String,
    /// A dry-run rule is evaluated and recorded but never fails the gate.
    #[serde(default)]
    pub dry_run: bool,
    /// Monitor rules: tags (`service:billing env:prod`, `team:*`) and name words every matching monitor has.
    #[serde(default)]
    pub query: String,
    /// Faulty deployment detection: the minutes of spans compared.
    #[serde(default = "default_window")]
    pub window_minutes: u32,
    /// Faulty deployment detection: percentage points the new version's error rate may exceed the previous one's.
    #[serde(default = "default_increase")]
    pub max_error_rate_increase: f64,
    /// Faulty deployment detection: requests the new version needs before it is judged.
    #[serde(default = "default_min_requests")]
    pub min_requests: u32,
}

fn default_window() -> u32 {
    30
}
fn default_increase() -> f64 {
    1.0
}
fn default_min_requests() -> u32 {
    50
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentGate {
    pub id: String,
    pub service: String,
    pub env: String,
    pub identifier: String,
    pub dry_run: bool,
    pub rules: Vec<GateRule>,
    pub created_by: String,
    pub updated_by: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    /// The latest evaluation's status and time, if any.
    pub last_status: Option<GateEvaluationStatus>,
    pub last_evaluated_at: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentGateInput {
    pub service: String,
    pub env: String,
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub dry_run: bool,
    pub rules: Vec<GateRule>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateRuleStatus {
    Passed,
    Failed,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateEvaluationStatus {
    Passed,
    Failed,
    /// Would have failed, but only dry-run rules (or a dry-run gate) failed: the deployment may go on.
    DryRun,
}

impl GateEvaluationStatus {
    fn as_str(self) -> &'static str {
        match self {
            GateEvaluationStatus::Passed => "passed",
            GateEvaluationStatus::Failed => "failed",
            GateEvaluationStatus::DryRun => "dry_run",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "failed" => GateEvaluationStatus::Failed,
            "dry_run" => GateEvaluationStatus::DryRun,
            _ => GateEvaluationStatus::Passed,
        }
    }
}

/// A monitor a monitor rule matched that was failing at evaluation time.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GateMonitorState {
    pub id: i64,
    pub name: String,
    /// ok | warn | alert | nodata, at evaluation time.
    pub status: String,
    /// Its status now (filled when one evaluation is read; None when the monitor was deleted).
    #[serde(default)]
    pub current_status: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GateRuleResult {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: GateRuleKind,
    pub status: GateRuleStatus,
    pub dry_run: bool,
    /// Why the rule passed or failed, in a sentence.
    pub reason: String,
    /// Monitor rules: how many monitors matched, and those in ALERT or NO DATA.
    #[serde(default)]
    pub matched_monitors: u32,
    #[serde(default)]
    pub monitors: Vec<GateMonitorState>,
    /// Faulty deployment detection: the versions compared and their error rates (0–1).
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub previous_version: Option<String>,
    #[serde(default)]
    pub error_rate: Option<f64>,
    #[serde(default)]
    pub previous_error_rate: Option<f64>,
    #[serde(default)]
    pub requests: Option<u64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GateEvaluationRequest {
    pub service: String,
    pub env: String,
    /// The gate's identifier; `default` when omitted.
    #[serde(default)]
    pub identifier: Option<String>,
    /// The version being deployed; the newest version seen in the window when omitted.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GateEvaluation {
    pub id: String,
    pub gate_id: Option<String>,
    pub service: String,
    pub env: String,
    pub identifier: String,
    pub version: String,
    pub status: GateEvaluationStatus,
    /// Whether the deployment may go on (passed or dry run): what a pipeline acts on.
    pub pass: bool,
    pub rules: Vec<GateRuleResult>,
    pub requested_by: String,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub duration_ms: i64,
}

/// A monitor as a monitor rule sees it.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct MonitorRow {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub tags: Vec<String>,
}

/// Spans of one version of the service (entry spans), over the window and the whole lookback.
#[derive(clickhouse::Row, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct VersionStats {
    pub version: String,
    pub hits_window: u64,
    pub errors_window: u64,
    pub hits_total: u64,
    pub errors_total: u64,
    pub first_seen: i64,
    pub last_seen: i64,
}

// ================================================================ evaluation logic

/// Whether a monitor matches a monitor rule's query: every `key:value` word is one of its tags
/// (`key:*` = any value of the key), every other word is in its name; case-insensitive.
pub(crate) fn monitor_matches(query: &str, name: &str, tags: &[String]) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return false;
    }
    let name = name.to_lowercase();
    let tags: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    words.iter().all(|w| match w.split_once(':') {
        Some((key, "*")) => tags.iter().any(|t| t.split_once(':').is_some_and(|(k, _)| k == key)),
        Some(_) => tags.iter().any(|t| t == w),
        None => name.contains(w.as_str()),
    })
}

fn empty_result(rule: &GateRule, status: GateRuleStatus, reason: String) -> GateRuleResult {
    GateRuleResult {
        name: rule.name.clone(),
        kind: rule.kind,
        status,
        dry_run: rule.dry_run,
        reason,
        matched_monitors: 0,
        monitors: Vec::new(),
        version: None,
        previous_version: None,
        error_rate: None,
        previous_error_rate: None,
        requests: None,
    }
}

/// A monitor rule's outcome over the monitors it matched.
pub(crate) fn monitor_outcome(rule: &GateRule, matched: &[MonitorRow]) -> GateRuleResult {
    let failing: Vec<GateMonitorState> =
        matched.iter().filter(|m| m.status == "alert" || m.status == "nodata").map(|m| GateMonitorState { id: m.id, name: m.name.clone(), status: m.status.clone(), current_status: None }).collect();
    let (status, reason) = if matched.is_empty() {
        (GateRuleStatus::Passed, format!("No monitor matches '{}'", rule.query))
    } else if failing.is_empty() {
        (GateRuleStatus::Passed, format!("None of the {} matching monitors is in ALERT or NO DATA", matched.len()))
    } else {
        (GateRuleStatus::Failed, format!("{} of the {} matching monitors are in ALERT or NO DATA", failing.len(), matched.len()))
    };
    GateRuleResult { matched_monitors: matched.len() as u32, monitors: failing, ..empty_result(rule, status, reason) }
}

/// The version being deployed and the one it replaces. The new one is `requested`, or the newest
/// version with spans in the window; the previous is the other version seen last.
pub(crate) fn pick_versions<'a>(stats: &'a [VersionStats], requested: Option<&str>) -> (Option<&'a VersionStats>, Option<&'a VersionStats>) {
    let requested = requested.map(str::trim).filter(|r| !r.is_empty());
    let new = match requested {
        Some(r) => stats.iter().find(|s| s.version == r),
        None => stats.iter().filter(|s| s.hits_window > 0).max_by_key(|s| s.first_seen),
    };
    let new_name = requested.or(new.map(|n| n.version.as_str()));
    let previous = stats.iter().filter(|s| Some(s.version.as_str()) != new_name).max_by_key(|s| s.last_seen);
    (new, previous)
}

fn rate(errors: u64, hits: u64) -> f64 {
    if hits == 0 {
        0.0
    } else {
        errors as f64 / hits as f64
    }
}

fn pct(r: f64) -> String {
    format!("{:.1}%", r * 100.0)
}

/// Faulty deployment detection: compares the new version's error rate with the previous one's.
pub(crate) fn faulty_outcome(rule: &GateRule, requested: Option<&str>, stats: &[VersionStats]) -> GateRuleResult {
    let (new, previous) = pick_versions(stats, requested);
    let Some(new) = new else {
        let reason = match requested.map(str::trim).filter(|r| !r.is_empty()) {
            Some(r) => format!("No spans of version {r} in the last {} minutes: nothing to judge yet", rule.window_minutes),
            None => format!("No versioned spans in the last {} minutes: nothing to judge yet", rule.window_minutes),
        };
        return empty_result(rule, GateRuleStatus::Passed, reason);
    };
    let error_rate = rate(new.errors_window, new.hits_window);
    let base = GateRuleResult { version: Some(new.version.clone()), error_rate: Some(error_rate), requests: Some(new.hits_window), ..empty_result(rule, GateRuleStatus::Passed, String::new()) };
    if new.hits_window < u64::from(rule.min_requests) {
        return GateRuleResult { reason: format!("Version {} served {} requests, fewer than the {} needed to judge it", new.version, new.hits_window, rule.min_requests), ..base };
    }
    let Some(previous) = previous else {
        return GateRuleResult { reason: format!("No previous version of the service to compare {} with", new.version), ..base };
    };
    let previous_rate = if previous.hits_window > 0 { rate(previous.errors_window, previous.hits_window) } else { rate(previous.errors_total, previous.hits_total) };
    let increase = (error_rate - previous_rate) * 100.0;
    let faulty = increase > rule.max_error_rate_increase;
    let reason = format!(
        "Error rate {} on {} vs {} on {} ({:+.1} points; {} +{} allowed)",
        pct(error_rate),
        new.version,
        pct(previous_rate),
        previous.version,
        increase,
        if faulty { "more than the" } else { "within the" },
        rule.max_error_rate_increase
    );
    GateRuleResult {
        status: if faulty { GateRuleStatus::Failed } else { GateRuleStatus::Passed },
        reason,
        previous_version: Some(previous.version.clone()),
        previous_error_rate: Some(previous_rate),
        ..base
    }
}

/// The gate's verdict: failed when a rule that isn't in dry run failed (and the gate isn't in dry
/// run), dry run when only dry-run rules failed, else passed.
pub(crate) fn verdict(gate_dry_run: bool, results: &[GateRuleResult]) -> GateEvaluationStatus {
    let failed: Vec<&GateRuleResult> = results.iter().filter(|r| r.status == GateRuleStatus::Failed).collect();
    if failed.iter().any(|r| !r.dry_run && !gate_dry_run) {
        GateEvaluationStatus::Failed
    } else if failed.is_empty() {
        GateEvaluationStatus::Passed
    } else {
        GateEvaluationStatus::DryRun
    }
}

/// `default`, `pre-deploy`, `canary_eu`: letters, digits, `-`, `_` and `.`.
fn valid_identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// Trims and checks a gate; fills rule names.
pub(crate) fn validate(input: &mut DeploymentGateInput) -> Result<(), String> {
    input.service = input.service.trim().to_string();
    input.env = input.env.trim().to_string();
    input.identifier = input.identifier.trim().to_string();
    if input.identifier.is_empty() {
        input.identifier = "default".into();
    }
    if input.service.is_empty() || input.service.chars().count() > MAX_FIELD_CHARS {
        return Err(format!("The service is required (at most {MAX_FIELD_CHARS} characters)"));
    }
    if input.env.is_empty() || input.env.chars().count() > MAX_FIELD_CHARS {
        return Err(format!("The environment is required (at most {MAX_FIELD_CHARS} characters)"));
    }
    if !valid_identifier(&input.identifier) {
        return Err("Identifiers are up to 100 letters, digits, '-', '_' and '.' (e.g. pre-deploy)".into());
    }
    if input.rules.is_empty() || input.rules.len() > MAX_RULES {
        return Err(format!("A gate has 1 to {MAX_RULES} rules"));
    }
    for (i, rule) in input.rules.iter_mut().enumerate() {
        rule.name = rule.name.trim().to_string();
        rule.query = rule.query.trim().to_string();
        if rule.name.is_empty() {
            rule.name = match rule.kind {
                GateRuleKind::Monitor => format!("Monitors {}", rule.query),
                GateRuleKind::FaultyDeployment => "Faulty deployment detection".into(),
            };
        }
        if rule.name.chars().count() > MAX_FIELD_CHARS {
            return Err(format!("Rule {}: names are at most {MAX_FIELD_CHARS} characters", i + 1));
        }
        match rule.kind {
            GateRuleKind::Monitor => {
                if rule.query.is_empty() || rule.query.chars().count() > MAX_QUERY_CHARS {
                    return Err(format!("Rule {}: the monitor query is required, e.g. service:billing env:prod", i + 1));
                }
            }
            GateRuleKind::FaultyDeployment => {
                if !(1..=1440).contains(&rule.window_minutes) {
                    return Err(format!("Rule {}: the window is 1 to 1440 minutes", i + 1));
                }
                if !rule.max_error_rate_increase.is_finite() || !(0.0..=100.0).contains(&rule.max_error_rate_increase) {
                    return Err(format!("Rule {}: the allowed error rate increase is 0 to 100 percentage points", i + 1));
                }
                if rule.min_requests > 1_000_000 {
                    return Err(format!("Rule {}: at most 1,000,000 minimum requests", i + 1));
                }
            }
        }
    }
    Ok(())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct GateRow {
    id: String,
    service: String,
    env: String,
    identifier: String,
    dry_run: bool,
    rules: serde_json::Value,
    created_by: String,
    updated_by: String,
    created_at: i64,
    updated_at: i64,
    last_status: Option<String>,
    last_evaluated_at: Option<i64>,
}

impl From<GateRow> for DeploymentGate {
    fn from(r: GateRow) -> Self {
        DeploymentGate {
            id: r.id,
            service: r.service,
            env: r.env,
            identifier: r.identifier,
            dry_run: r.dry_run,
            rules: serde_json::from_value(r.rules).unwrap_or_default(),
            created_by: r.created_by,
            updated_by: r.updated_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
            last_status: r.last_status.as_deref().map(GateEvaluationStatus::parse),
            last_evaluated_at: r.last_evaluated_at,
        }
    }
}

const GATE_SELECT: &str = "SELECT g.id::text AS id, g.service, g.env, g.identifier, g.dry_run, g.rules, g.created_by, g.updated_by, \
     (extract(epoch FROM g.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM g.updated_at) * 1000)::bigint AS updated_at, \
     l.status AS last_status, (extract(epoch FROM l.started_at) * 1000)::bigint AS last_evaluated_at \
     FROM deployment_gates g LEFT JOIN LATERAL (SELECT e.status, e.started_at FROM deployment_gate_evaluations e \
     WHERE e.org_id = g.org_id AND e.gate_id = g.id ORDER BY e.started_at DESC LIMIT 1) l ON true";

async fn get_gate(st: &AppState, org: &str, id: &str) -> Result<DeploymentGate, ApiError> {
    let row: Option<GateRow> = sqlx::query_as(&format!("{GATE_SELECT} WHERE g.org_id = $1 AND g.id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(DeploymentGate::from).ok_or_else(|| ApiError::NotFound("Deployment gate not found".into()))
}

fn unique(e: sqlx::Error, input: &DeploymentGateInput) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A gate for {} in {} with the identifier '{}' already exists", input.service, input.env, input.identifier)),
        _ => e.into(),
    }
}

fn describe(input: &DeploymentGateInput) -> String {
    format!("{} / {} / {}", input.service, input.env, input.identifier)
}

#[derive(sqlx::FromRow)]
struct EvaluationRow {
    id: String,
    gate_id: Option<String>,
    service: String,
    env: String,
    identifier: String,
    version: String,
    status: String,
    rule_results: serde_json::Value,
    requested_by: String,
    started_at: i64,
    duration_ms: i32,
}

impl From<EvaluationRow> for GateEvaluation {
    fn from(r: EvaluationRow) -> Self {
        let status = GateEvaluationStatus::parse(&r.status);
        GateEvaluation {
            id: r.id,
            gate_id: r.gate_id,
            service: r.service,
            env: r.env,
            identifier: r.identifier,
            version: r.version,
            pass: status != GateEvaluationStatus::Failed,
            status,
            rules: serde_json::from_value(r.rule_results).unwrap_or_default(),
            requested_by: r.requested_by,
            started_at: r.started_at,
            duration_ms: i64::from(r.duration_ms),
        }
    }
}

const EVALUATION_COLS: &str = "id::text AS id, gate_id::text AS gate_id, service, env, identifier, version, status, rule_results, requested_by, \
     (extract(epoch FROM started_at) * 1000)::bigint AS started_at, duration_ms";

/// Spans of each version of the service in the environment over the lookback, with the window's counts.
async fn version_stats(st: &AppState, org: &str, service: &str, env: &str, window_minutes: u32, now_ms: i64) -> Result<Vec<VersionStats>, ApiError> {
    let window_from = now_ms - i64::from(window_minutes) * 60_000;
    let lookback_from = now_ms - LOOKBACK_MS.max(i64::from(window_minutes) * 60_000);
    let sql = format!(
        "SELECT ResourceAttributes['service.version'] AS version, \
         countIf(Timestamp >= fromUnixTimestamp64Milli(?)) AS hits_window, countIf(Timestamp >= fromUnixTimestamp64Milli(?) AND StatusCode = 'Error') AS errors_window, \
         count() AS hits_total, countIf(StatusCode = 'Error') AS errors_total, \
         toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_seen \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
         AND ServiceName = ? AND ResourceAttributes['deployment.environment'] = ? AND {ENTRY} AND ResourceAttributes['service.version'] != '' \
         GROUP BY version"
    );
    Ok(st.ch(org).query(&sql).bind(window_from).bind(window_from).bind(lookback_from).bind(now_ms).bind(service).bind(env).fetch_all::<VersionStats>().await?)
}

/// Evaluates every rule of a gate now.
async fn evaluate_rules(st: &AppState, org: &str, gate: &DeploymentGate, version: Option<&str>, now_ms: i64) -> Result<Vec<GateRuleResult>, ApiError> {
    let monitors: Vec<MonitorRow> = if gate.rules.iter().any(|r| r.kind == GateRuleKind::Monitor) {
        sqlx::query_as("SELECT id, name, status, tags FROM monitors WHERE org_id = $1 AND NOT draft ORDER BY id").bind(org).fetch_all(&st.postgres).await?
    } else {
        Vec::new()
    };
    let mut results = Vec::with_capacity(gate.rules.len());
    for rule in &gate.rules {
        results.push(match rule.kind {
            GateRuleKind::Monitor => {
                let matched: Vec<MonitorRow> = monitors.iter().filter(|m| monitor_matches(&rule.query, &m.name, &m.tags)).cloned().collect();
                monitor_outcome(rule, &matched)
            }
            GateRuleKind::FaultyDeployment => {
                let stats = version_stats(st, org, &gate.service, &gate.env, rule.window_minutes, now_ms).await?;
                faulty_outcome(rule, version, &stats)
            }
        });
    }
    Ok(results)
}

// ================================================================ API

#[utoipa::path(get, path = "/api/v1/deployment-gates", operation_id = "list_deployment_gates", responses((status = 200, body = Vec<DeploymentGate>)), tag = "delivery")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<DeploymentGate>> {
    let rows: Vec<GateRow> = sqlx::query_as(&format!("{GATE_SELECT} WHERE g.org_id = $1 ORDER BY g.service, g.env, g.identifier")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(DeploymentGate::from).collect()))
}

#[utoipa::path(get, path = "/api/v1/deployment-gates/{id}", operation_id = "get_deployment_gate", params(("id" = String, Path)), responses((status = 200, body = DeploymentGate)), tag = "delivery")]
pub async fn get(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<DeploymentGate> {
    Ok(Json(get_gate(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/deployment-gates", operation_id = "create_deployment_gate", request_body = DeploymentGateInput, responses((status = 200, body = DeploymentGate)), tag = "delivery")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<DeploymentGateInput>) -> ApiResult<DeploymentGate> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM deployment_gates WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_GATES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_GATES} deployment gates")));
    }
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO deployment_gates (org_id, service, env, identifier, dry_run, rules, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&input.service)
            .bind(&input.env)
            .bind(&input.identifier)
            .bind(input.dry_run)
            .bind(serde_json::json!(input.rules))
            .bind(&ctx.name)
            .fetch_one(&st.postgres)
            .await
            .map_err(|e| unique(e, &input))?;
    ctx.audit(&st, "deployment_gate.created", &describe(&input), &format!("{} rules", input.rules.len())).await;
    Ok(Json(get_gate(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/deployment-gates/{id}", operation_id = "update_deployment_gate", params(("id" = String, Path)), request_body = DeploymentGateInput, responses((status = 200, body = DeploymentGate)), tag = "delivery")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<DeploymentGateInput>) -> ApiResult<DeploymentGate> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let done = sqlx::query("UPDATE deployment_gates SET service = $3, env = $4, identifier = $5, dry_run = $6, rules = $7, updated_by = $8, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.service)
        .bind(&input.env)
        .bind(&input.identifier)
        .bind(input.dry_run)
        .bind(serde_json::json!(input.rules))
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await
        .map_err(|e| unique(e, &input))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Deployment gate not found".into()));
    }
    ctx.audit(&st, "deployment_gate.updated", &describe(&input), &format!("{} rules, dry run {}", input.rules.len(), input.dry_run)).await;
    Ok(Json(get_gate(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/deployment-gates/{id}", operation_id = "delete_deployment_gate", params(("id" = String, Path)), responses((status = 204)), tag = "delivery")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("DELETE FROM deployment_gates WHERE org_id = $1 AND id::text = $2 RETURNING service, env, identifier").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((service, env, identifier)) = row else { return Err(ApiError::NotFound("Deployment gate not found".into())) };
    ctx.audit(&st, "deployment_gate.deleted", &format!("{service} / {env} / {identifier}"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Evaluates the gate of a service, environment and identifier now, records the evaluation and
/// answers with its verdict. Called by CI/CD pipelines with an application key
/// (`Authorization: Bearer ocak_…`); `pass` says whether the deployment may go on.
#[utoipa::path(post, path = "/api/v1/deployment-gates/evaluate", operation_id = "evaluate_deployment_gate", request_body = GateEvaluationRequest, responses((status = 200, body = GateEvaluation), (status = 404, description = "No gate for this service, environment and identifier")), tag = "delivery")]
pub async fn evaluate(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<GateEvaluationRequest>) -> ApiResult<GateEvaluation> {
    let started = Instant::now();
    let service = req.service.trim();
    let env = req.env.trim();
    let identifier = req.identifier.as_deref().map(str::trim).filter(|i| !i.is_empty()).unwrap_or("default");
    let version = req.version.as_deref().map(str::trim).filter(|v| !v.is_empty());
    if service.is_empty() || env.is_empty() {
        return Err(ApiError::BadRequest("service and env are required".into()));
    }
    if version.is_some_and(|v| v.chars().count() > MAX_FIELD_CHARS) {
        return Err(ApiError::BadRequest(format!("version is at most {MAX_FIELD_CHARS} characters")));
    }
    let row: Option<GateRow> = sqlx::query_as(&format!("{GATE_SELECT} WHERE g.org_id = $1 AND g.service = $2 AND g.env = $3 AND g.identifier = $4"))
        .bind(&ctx.org_id)
        .bind(service)
        .bind(env)
        .bind(identifier)
        .fetch_optional(&st.postgres)
        .await?;
    let Some(gate) = row.map(DeploymentGate::from) else {
        return Err(ApiError::NotFound(format!("No deployment gate for {service} in {env} with the identifier '{identifier}'")));
    };
    let now_ms = super::metrics::now_ms();
    let results = evaluate_rules(&st, &ctx.org_id, &gate, version, now_ms).await?;
    let status = verdict(gate.dry_run, &results);
    let duration_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO deployment_gate_evaluations (org_id, gate_id, service, env, identifier, version, status, rule_results, requested_by, duration_ms) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&gate.id)
    .bind(&gate.service)
    .bind(&gate.env)
    .bind(&gate.identifier)
    .bind(version.unwrap_or(""))
    .bind(status.as_str())
    .bind(serde_json::json!(results))
    .bind(&ctx.name)
    .bind(duration_ms)
    .fetch_one(&st.postgres)
    .await?;
    sqlx::query(&format!("DELETE FROM deployment_gate_evaluations WHERE org_id = $1 AND started_at < now() - interval '{RETENTION_DAYS} days'")).bind(&ctx.org_id).execute(&st.postgres).await?;
    let row: EvaluationRow =
        sqlx::query_as(&format!("SELECT {EVALUATION_COLS} FROM deployment_gate_evaluations WHERE org_id = $1 AND id::text = $2")).bind(&ctx.org_id).bind(&id).fetch_one(&st.postgres).await?;
    Ok(Json(row.into()))
}

/// The org's latest evaluations, newest first.
#[utoipa::path(get, path = "/api/v1/deployment-gates/evaluations", operation_id = "list_deployment_gate_evaluations", responses((status = 200, body = Vec<GateEvaluation>)), tag = "delivery")]
pub async fn evaluations(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<GateEvaluation>> {
    let rows: Vec<EvaluationRow> = sqlx::query_as(&format!("SELECT {EVALUATION_COLS} FROM deployment_gate_evaluations WHERE org_id = $1 ORDER BY started_at DESC LIMIT $2"))
        .bind(&ctx.org_id)
        .bind(MAX_EVALUATIONS)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(rows.into_iter().map(GateEvaluation::from).collect()))
}

/// One evaluation, with the current status of the monitors that failed it.
#[utoipa::path(get, path = "/api/v1/deployment-gates/evaluations/{id}", operation_id = "get_deployment_gate_evaluation", params(("id" = String, Path)), responses((status = 200, body = GateEvaluation)), tag = "delivery")]
pub async fn evaluation(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<GateEvaluation> {
    let row: Option<EvaluationRow> =
        sqlx::query_as(&format!("SELECT {EVALUATION_COLS} FROM deployment_gate_evaluations WHERE org_id = $1 AND id::text = $2")).bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some(mut evaluation) = row.map(GateEvaluation::from) else { return Err(ApiError::NotFound("Evaluation not found".into())) };
    let ids: Vec<i64> = evaluation.rules.iter().flat_map(|r| r.monitors.iter().map(|m| m.id)).collect();
    if !ids.is_empty() {
        let now: Vec<(i64, String)> = sqlx::query_as("SELECT id, status FROM monitors WHERE org_id = $1 AND id = ANY($2)").bind(&ctx.org_id).bind(&ids).fetch_all(&st.postgres).await?;
        for m in evaluation.rules.iter_mut().flat_map(|r| r.monitors.iter_mut()) {
            m.current_status = now.iter().find(|(id, _)| *id == m.id).map(|(_, s)| s.clone());
        }
    }
    Ok(Json(evaluation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn monitor_rule(query: &str) -> GateRule {
        GateRule { kind: GateRuleKind::Monitor, name: "Monitors".into(), dry_run: false, query: query.into(), window_minutes: 30, max_error_rate_increase: 1.0, min_requests: 50 }
    }

    fn faulty_rule() -> GateRule {
        GateRule { kind: GateRuleKind::FaultyDeployment, name: "Faulty deployment".into(), dry_run: false, query: String::new(), window_minutes: 30, max_error_rate_increase: 1.0, min_requests: 50 }
    }

    fn monitor(id: i64, status: &str) -> MonitorRow {
        MonitorRow { id, name: format!("monitor {id}"), status: status.into(), tags: tags(&["service:billing"]) }
    }

    fn v(version: &str, hits: u64, errors: u64, first: i64, last: i64) -> VersionStats {
        VersionStats { version: version.into(), hits_window: hits, errors_window: errors, hits_total: hits, errors_total: errors, first_seen: first, last_seen: last }
    }

    #[test]
    fn monitors_match_on_tags_and_name_words() {
        let t = tags(&["service:billing", "env:prod", "team:payments"]);
        assert!(monitor_matches("service:billing env:prod", "High latency", &t));
        assert!(monitor_matches("SERVICE:Billing", "High latency", &t), "case-insensitive");
        assert!(monitor_matches("team:*", "High latency", &t), "any value of a key");
        assert!(monitor_matches("service:billing latency", "High latency on checkout", &t), "plain words match the name");
        assert!(!monitor_matches("service:billing env:staging", "High latency", &t));
        assert!(!monitor_matches("service:billing errors", "High latency", &t));
        assert!(!monitor_matches("region:*", "High latency", &t));
        assert!(!monitor_matches("  ", "High latency", &t), "an empty query matches nothing");
    }

    #[test]
    fn a_monitor_rule_fails_on_alert_or_no_data() {
        let r = monitor_outcome(&monitor_rule("service:billing"), &[monitor(1, "ok"), monitor(2, "warn")]);
        assert_eq!(r.status, GateRuleStatus::Passed);
        assert_eq!(r.matched_monitors, 2);
        assert!(r.monitors.is_empty());

        let r = monitor_outcome(&monitor_rule("service:billing"), &[monitor(1, "ok"), monitor(2, "alert"), monitor(3, "nodata")]);
        assert_eq!(r.status, GateRuleStatus::Failed);
        assert_eq!(r.monitors.iter().map(|m| m.id).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(r.monitors[0].status, "alert");
        assert!(r.reason.contains('2'), "{}", r.reason);

        let none = monitor_outcome(&monitor_rule("service:billing"), &[]);
        assert_eq!(none.status, GateRuleStatus::Passed);
        assert!(none.reason.to_lowercase().contains("no monitor"), "{}", none.reason);
    }

    #[test]
    fn versions_are_the_requested_or_newest_and_the_one_seen_before() {
        let stats = [v("1.4.0", 500, 5, 1_000, 9_000), v("1.5.0", 200, 2, 8_000, 10_000), v("1.3.0", 0, 0, 100, 2_000)];
        let (new, prev) = pick_versions(&stats, None);
        assert_eq!(new.unwrap().version, "1.5.0");
        assert_eq!(prev.unwrap().version, "1.4.0");
        let (new, prev) = pick_versions(&stats, Some("1.4.0"));
        assert_eq!(new.unwrap().version, "1.4.0");
        assert_eq!(prev.unwrap().version, "1.5.0", "the other version seen last");
        let (new, prev) = pick_versions(&stats, Some("2.0.0"));
        assert!(new.is_none());
        assert_eq!(prev.unwrap().version, "1.5.0");
        assert_eq!(pick_versions(&[], None), (None, None));
        // A version without spans in the window isn't the newest deployment.
        let idle = [v("1.0", 10, 0, 1, 2), VersionStats { hits_window: 0, ..v("0.9", 10, 0, 5, 6) }];
        let (new, _) = pick_versions(&idle, None);
        assert_eq!(new.unwrap().version, "1.0");
    }

    #[test]
    fn faulty_deployment_compares_error_rates() {
        let rule = faulty_rule();
        // 1% → 1.5%: within +1 point.
        let ok = faulty_outcome(&rule, Some("1.5.0"), &[v("1.4.0", 1000, 10, 0, 9), v("1.5.0", 200, 3, 5, 10)]);
        assert_eq!(ok.status, GateRuleStatus::Passed);
        assert_eq!(ok.version.as_deref(), Some("1.5.0"));
        assert_eq!(ok.previous_version.as_deref(), Some("1.4.0"));
        assert_eq!(ok.error_rate, Some(0.015));
        assert_eq!(ok.previous_error_rate, Some(0.01));
        // 1% → 5%: faulty.
        let bad = faulty_outcome(&rule, Some("1.5.0"), &[v("1.4.0", 1000, 10, 0, 9), v("1.5.0", 200, 10, 5, 10)]);
        assert_eq!(bad.status, GateRuleStatus::Failed);
        assert!(bad.reason.contains("5.0%"), "{}", bad.reason);
        // Not enough requests yet: passes, says so.
        let few = faulty_outcome(&rule, Some("1.5.0"), &[v("1.4.0", 1000, 10, 0, 9), v("1.5.0", 20, 10, 5, 10)]);
        assert_eq!(few.status, GateRuleStatus::Passed);
        assert!(few.reason.contains("20"), "{}", few.reason);
        // Nothing to compare with.
        let first = faulty_outcome(&rule, None, &[v("1.0.0", 1000, 900, 0, 9)]);
        assert_eq!(first.status, GateRuleStatus::Passed);
        assert!(first.previous_version.is_none());
        // No spans of the version.
        let missing = faulty_outcome(&rule, Some("9.9.9"), &[v("1.0.0", 1000, 1, 0, 9)]);
        assert_eq!(missing.status, GateRuleStatus::Passed);
        assert!(missing.reason.contains("9.9.9"), "{}", missing.reason);
        // The previous version, gone from the window, is judged on the whole lookback.
        let gone = VersionStats { hits_window: 0, errors_window: 0, hits_total: 1000, errors_total: 10, ..v("1.4.0", 0, 0, 0, 3) };
        let r = faulty_outcome(&rule, Some("1.5.0"), &[gone, v("1.5.0", 500, 50, 5, 10)]);
        assert_eq!(r.status, GateRuleStatus::Failed);
        assert_eq!(r.previous_error_rate, Some(0.01));
    }

    #[test]
    fn the_verdict_honours_dry_runs() {
        let result = |status, dry_run| GateRuleResult { status, dry_run, ..monitor_outcome(&monitor_rule("x:y"), &[]) };
        assert_eq!(verdict(false, &[result(GateRuleStatus::Passed, false)]), GateEvaluationStatus::Passed);
        assert_eq!(verdict(false, &[]), GateEvaluationStatus::Passed);
        assert_eq!(verdict(false, &[result(GateRuleStatus::Passed, false), result(GateRuleStatus::Failed, false)]), GateEvaluationStatus::Failed);
        assert_eq!(verdict(false, &[result(GateRuleStatus::Failed, true)]), GateEvaluationStatus::DryRun);
        assert_eq!(verdict(true, &[result(GateRuleStatus::Failed, false)]), GateEvaluationStatus::DryRun);
        assert_eq!(verdict(false, &[result(GateRuleStatus::Failed, true), result(GateRuleStatus::Failed, false)]), GateEvaluationStatus::Failed);
    }

    #[test]
    fn gates_are_checked() {
        let mut ok = DeploymentGateInput { service: " billing ".into(), env: "prod".into(), identifier: "".into(), dry_run: false, rules: vec![monitor_rule(" service:billing "), faulty_rule()] };
        ok.rules[0].name = "".into();
        validate(&mut ok).unwrap();
        assert_eq!(ok.service, "billing");
        assert_eq!(ok.identifier, "default");
        assert_eq!(ok.rules[0].query, "service:billing");
        assert!(!ok.rules[0].name.is_empty(), "rules get a default name");

        let base = || DeploymentGateInput { service: "billing".into(), env: "prod".into(), identifier: "default".into(), dry_run: false, rules: vec![faulty_rule()] };
        let mut no_service = DeploymentGateInput { service: " ".into(), ..base() };
        assert!(validate(&mut no_service).is_err());
        let mut no_env = DeploymentGateInput { env: "".into(), ..base() };
        assert!(validate(&mut no_env).is_err());
        let mut bad_id = DeploymentGateInput { identifier: "pre deploy!".into(), ..base() };
        assert!(validate(&mut bad_id).is_err());
        let mut no_rules = DeploymentGateInput { rules: vec![], ..base() };
        assert!(validate(&mut no_rules).is_err());
        let mut empty_query = DeploymentGateInput { rules: vec![monitor_rule("")], ..base() };
        assert!(validate(&mut empty_query).is_err());
        let mut window = base();
        window.rules[0].window_minutes = 0;
        assert!(validate(&mut window).is_err());
        let mut increase = base();
        increase.rules[0].max_error_rate_increase = -1.0;
        assert!(validate(&mut increase).is_err());
    }
}
