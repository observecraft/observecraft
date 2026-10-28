//! Developer Portal → Scorecards: rules evaluated per service, grouped into scorecards, with levels.
//!
//! - **Services** are the instrumented APM services (entry spans of the past 7 days) plus every
//!   service with metadata (team, contacts, code repository, docs), which users edit here.
//! - **Rules.** Setting scorecards up creates the three recommended scorecards with the built-in
//!   rules picked (see [`super::scorecard_rules`]); custom rules get their outcomes per service
//!   from users or the API (`POST /api/v1/scorecards/rules/{id}/outcomes`). Every rule has a level
//!   (1–3) and a scope (a service search); a service reaches a level when no rule at or below it
//!   fails.
//! - **Evaluation.** A full evaluation reads the org's spans, logs and CI deployments (ClickHouse,
//!   through `AppState::ch(org)`), keeps those facts as a snapshot, and combines them with monitors,
//!   SLOs and metadata (Postgres) into one outcome per (rule, service in scope), replacing the
//!   previous ones. It runs hourly ([`run_scheduler`], orgs claimed with `FOR UPDATE SKIP LOCKED`)
//!   and on "Evaluate now"; edits to rules, outcomes or metadata re-evaluate at once from the
//!   snapshot. Only standard users and admins change anything.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::apm::ENTRY;
use super::logs::{bind, scope, TimeRange};
use super::scorecard_rules::{self as rules, BuiltinRule, OrgFacts, Scope, ScorecardOutcomeState, ServiceFacts, Verdict, VersionSeen, DAY_MS};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::AppState;

const EVALUATION_INTERVAL_HOURS: i32 = 1;
/// How long a claimed evaluation may run before another server may take the org over.
const CLAIM_SECONDS: f64 = 600.0;
const MAX_CUSTOM_RULES: i64 = 200;
const MAX_SCORECARDS: i64 = 50;
const MAX_NAME_CHARS: usize = 200;
const MAX_SCORECARD_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 2000;
const MAX_SCOPE_CHARS: usize = 500;
const MAX_SERVICE_CHARS: usize = 200;
const MAX_OUTCOMES: usize = 1000;
const MAX_REMARKS_CHARS: usize = 500;
const MAX_CONTACTS: usize = 20;
const MAX_URL_CHARS: usize = 2048;
const MAX_TEAM_CHARS: usize = 100;
const CUSTOM_NO_OUTCOME: &str = "No outcome reported for this service";

// ================================================================ API types

/// How to reach a service's owners.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ServiceContact {
    /// email | slack | link
    pub r#type: String,
    #[serde(default)]
    pub name: String,
    /// Address, channel or URL.
    pub value: String,
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMetadataInput {
    /// Team handle; empty when no team owns the service.
    #[serde(default)]
    pub team: String,
    #[serde(default)]
    pub contacts: Vec<ServiceContact>,
    /// http(s) URL of the code repository.
    #[serde(default)]
    pub repo_url: String,
    /// http(s) URL of the documentation or runbook.
    #[serde(default)]
    pub docs_url: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scorecard {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardRule {
    pub id: String,
    pub scorecard_id: String,
    /// Key of a built-in rule (`monitors_defined`, …); null for a custom rule.
    pub builtin: Option<String>,
    pub name: String,
    pub description: String,
    /// 1 (basic) – 3 (advanced).
    pub level: u8,
    /// Service search the rule applies to (`service:order-* team:checkout`); empty for every service.
    pub scope: String,
    pub enabled: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardOutcome {
    pub rule_id: String,
    pub service: String,
    pub state: ScorecardOutcomeState,
    pub reason: String,
}

/// A service with its metadata and levels.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardService {
    pub service: String,
    pub team: String,
    pub contacts: Vec<ServiceContact>,
    pub repo_url: String,
    pub docs_url: String,
    /// Seen on spans of the past 7 days (otherwise only declared in metadata).
    pub instrumented: bool,
    /// Level across every enabled rule (0–3).
    pub level: u8,
    /// Level per scorecard id.
    pub levels: BTreeMap<String, u8>,
    /// Passing share of passed and failed rules, in percent.
    pub score: Option<f64>,
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
}

/// A built-in rule offered when setting scorecards up.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardRecommendation {
    pub key: String,
    pub name: String,
    pub description: String,
    pub level: u8,
    pub scorecard: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardsOverview {
    /// False until scorecards are set up (the first-run page).
    pub set_up: bool,
    /// Last evaluation (epoch ms).
    pub evaluated_at: Option<i64>,
    /// When telemetry was last read (epoch ms).
    pub telemetry_at: Option<i64>,
    pub next_evaluation_at: Option<i64>,
    pub scorecards: Vec<Scorecard>,
    pub rules: Vec<ScorecardRule>,
    pub outcomes: Vec<ScorecardOutcome>,
    pub services: Vec<ScorecardService>,
    pub recommended: Vec<ScorecardRecommendation>,
}

#[derive(Deserialize, ToSchema)]
pub struct ScorecardSetupInput {
    /// Keys of the built-in rules to enable; the others are created disabled.
    pub rules: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardRuleInput {
    pub name: String,
    /// Scorecard name; created when it doesn't exist.
    pub scorecard: String,
    #[serde(default)]
    pub description: String,
    pub level: u8,
    #[serde(default)]
    pub scope: String,
}

/// Changes to a rule; fields left out stay. Built-in rules take only level, scope and enabled.
#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardRuleUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub level: Option<u8>,
    pub scope: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct ScorecardOutcomeInput {
    pub service: String,
    pub state: ScorecardOutcomeState,
    #[serde(default)]
    pub remarks: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ScorecardOutcomesInput {
    pub outcomes: Vec<ScorecardOutcomeInput>,
}

// ================================================================ evaluation (pure)

/// Telemetry facts of one service, kept between full evaluations.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
struct Telemetry {
    #[serde(default)]
    versions: Vec<VersionSeen>,
    #[serde(default)]
    logs: u64,
    #[serde(default)]
    traced_logs: u64,
    #[serde(default)]
    last_ci_deploy: Option<i64>,
}

impl Telemetry {
    fn instrumented(&self) -> bool {
        self.versions.iter().any(|v| v.recent_spans > 0)
    }
}

#[derive(Clone, Debug, Default)]
struct Metadata {
    team: String,
    contacts: Vec<ServiceContact>,
    repo_url: String,
    docs_url: String,
}

/// A rule as the evaluator sees it.
#[derive(Clone, Debug)]
struct RuleDef {
    id: String,
    scorecard_id: String,
    builtin: Option<String>,
    level: u8,
    scope: String,
    enabled: bool,
}

/// An SLO as the monitors and SLOs rules see it.
#[derive(Clone, Debug, Default)]
struct SloRef {
    name: String,
    /// Good, total or slice queries.
    queries: Vec<String>,
    tags: Vec<String>,
    /// Names of the monitors of a monitor SLO.
    monitors: Vec<String>,
}

/// Everything an evaluation reads, gathered from storage.
#[derive(Default)]
struct Inputs {
    rules: Vec<RuleDef>,
    telemetry: BTreeMap<String, Telemetry>,
    metadata: BTreeMap<String, Metadata>,
    /// (name, query, tags) of the org's monitors.
    monitors: Vec<(String, String, Vec<String>)>,
    slos: Vec<SloRef>,
    /// rule id → service → (state, remarks).
    manual: HashMap<String, HashMap<String, (ScorecardOutcomeState, String)>>,
    org: OrgFacts,
}

/// The services scorecards cover: instrumented ones and those with metadata, by name.
fn service_names(inputs: &Inputs) -> Vec<String> {
    let mut names: Vec<String> = inputs.telemetry.iter().filter(|(_, t)| t.instrumented()).map(|(s, _)| s.clone()).collect();
    names.extend(inputs.metadata.keys().cloned());
    names.sort();
    names.dedup();
    names
}

fn facts_for(inputs: &Inputs, service: &str) -> ServiceFacts {
    let meta = inputs.metadata.get(service).cloned().unwrap_or_default();
    let tel = inputs.telemetry.get(service).cloned().unwrap_or_default();
    let mentions = |text: &str| rules::references_service(text, service);
    let monitors: Vec<String> = inputs.monitors.iter().filter(|(_, q, tags)| mentions(q) || tags.iter().any(|t| mentions(t))).map(|(n, _, _)| n.clone()).collect();
    let slos = inputs.slos.iter().filter(|slo| slo.queries.iter().chain(&slo.tags).any(|t| mentions(t)) || slo.monitors.iter().any(|m| monitors.contains(m))).map(|slo| slo.name.clone()).collect();
    ServiceFacts {
        team: meta.team,
        contacts: meta.contacts.len(),
        repo_url: meta.repo_url,
        docs_url: meta.docs_url,
        versions: tel.versions,
        logs: tel.logs,
        traced_logs: tel.traced_logs,
        last_ci_deploy: tel.last_ci_deploy,
        monitors,
        slos,
    }
}

/// One outcome per (enabled rule, service in its scope).
fn evaluate_all(inputs: &Inputs) -> Vec<ScorecardOutcome> {
    let services = service_names(inputs);
    let facts: Vec<(String, ServiceFacts)> = services.iter().map(|s| (s.clone(), facts_for(inputs, s))).collect();
    let mut out = Vec::new();
    for rule in inputs.rules.iter().filter(|r| r.enabled) {
        let scope = Scope::parse(&rule.scope);
        let builtin = rule.builtin.as_deref().and_then(BuiltinRule::from_key);
        for (service, f) in &facts {
            if !scope.covers(service, &f.team) {
                continue;
            }
            let Verdict { state, reason } = match builtin {
                Some(b) => rules::evaluate(b, f, &inputs.org),
                None => match inputs.manual.get(&rule.id).and_then(|m| m.get(service)) {
                    Some((state, remarks)) => Verdict { state: *state, reason: remarks.clone() },
                    None => Verdict { state: ScorecardOutcomeState::Skip, reason: CUSTOM_NO_OUTCOME.into() },
                },
            };
            out.push(ScorecardOutcome { rule_id: rule.id.clone(), service: service.clone(), state, reason });
        }
    }
    out
}

/// Levels and counts per service from the outcomes.
fn score_services(inputs: &Inputs, scorecards: &[Scorecard], outcomes: &[ScorecardOutcome]) -> Vec<ScorecardService> {
    let rule_by_id: HashMap<&str, &RuleDef> = inputs.rules.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut by_service: HashMap<&str, Vec<(&RuleDef, ScorecardOutcomeState)>> = HashMap::new();
    for o in outcomes {
        if let Some(rule) = rule_by_id.get(o.rule_id.as_str()) {
            by_service.entry(o.service.as_str()).or_default().push((rule, o.state));
        }
    }
    service_names(inputs)
        .into_iter()
        .map(|service| {
            let meta = inputs.metadata.get(&service).cloned().unwrap_or_default();
            let mine = by_service.get(service.as_str()).cloned().unwrap_or_default();
            let all: Vec<(u8, ScorecardOutcomeState)> = mine.iter().map(|(r, s)| (r.level, *s)).collect();
            let levels = scorecards
                .iter()
                .map(|c| {
                    let of_card: Vec<(u8, ScorecardOutcomeState)> = mine.iter().filter(|(r, _)| r.scorecard_id == c.id).map(|(r, s)| (r.level, *s)).collect();
                    (c.id.clone(), rules::service_level(&of_card))
                })
                .collect();
            let count = |state| all.iter().filter(|(_, s)| *s == state).count();
            ScorecardService {
                instrumented: inputs.telemetry.get(&service).is_some_and(Telemetry::instrumented),
                level: rules::service_level(&all),
                score: rules::score(&all).map(|s| (s * 10.0).round() / 10.0),
                pass: count(ScorecardOutcomeState::Pass),
                fail: count(ScorecardOutcomeState::Fail),
                skip: count(ScorecardOutcomeState::Skip),
                levels,
                team: meta.team,
                contacts: meta.contacts,
                repo_url: meta.repo_url,
                docs_url: meta.docs_url,
                service,
            }
        })
        .collect()
}

fn recommendations() -> Vec<ScorecardRecommendation> {
    BuiltinRule::ALL
        .iter()
        .map(|r| ScorecardRecommendation { key: r.key().into(), name: r.name().into(), description: r.description().into(), level: r.level(), scorecard: r.scorecard().into() })
        .collect()
}

// ================================================================ validation (pure)

fn http_url(url: &str, what: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    if url.chars().count() > MAX_URL_CHARS {
        return Err(format!("The {what} is too long"));
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) || url.chars().any(char::is_whitespace) {
        return Err(format!("The {what} must be an http(s) URL"));
    }
    Ok(url.to_string())
}

fn check_metadata(input: &ServiceMetadataInput) -> Result<Metadata, String> {
    let team = input.team.trim().trim_start_matches('@').to_string();
    if team.chars().count() > MAX_TEAM_CHARS {
        return Err("The team handle is too long".into());
    }
    if input.contacts.len() > MAX_CONTACTS {
        return Err(format!("A service can have at most {MAX_CONTACTS} contacts"));
    }
    let mut contacts = Vec::new();
    for c in &input.contacts {
        let value = c.value.trim();
        if value.is_empty() {
            continue;
        }
        let value = match c.r#type.as_str() {
            "email" if value.contains('@') && !value.contains(char::is_whitespace) && value.len() <= 320 => value.to_string(),
            "email" => return Err(format!("{value} is not an email address")),
            "slack" if value.len() <= 200 => value.to_string(),
            "link" => http_url(value, "contact link")?,
            "slack" => return Err("The chat channel is too long".into()),
            other => return Err(format!("Unknown contact type {other}: use email, slack or link")),
        };
        contacts.push(ServiceContact { r#type: c.r#type.clone(), name: c.name.trim().chars().take(100).collect(), value });
    }
    Ok(Metadata { team, contacts, repo_url: http_url(&input.repo_url, "code repository")?, docs_url: http_url(&input.docs_url, "documentation link")? })
}

fn check_service(service: &str) -> Result<String, String> {
    let s = service.trim();
    if s.is_empty() || s.chars().count() > MAX_SERVICE_CHARS {
        return Err(format!("A service name has 1 to {MAX_SERVICE_CHARS} characters"));
    }
    Ok(s.to_string())
}

fn check_text(value: &str, what: &str, max: usize, required: bool) -> Result<String, String> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(format!("The {what} is required"));
    }
    if v.chars().count() > max {
        return Err(format!("The {what} has at most {max} characters"));
    }
    Ok(v.to_string())
}

fn check_level(level: u8) -> Result<u8, String> {
    if (1..=rules::MAX_LEVEL).contains(&level) {
        Ok(level)
    } else {
        Err("The level is 1, 2 or 3".into())
    }
}

fn valid_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

// ================================================================ storage

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(sqlx::FromRow)]
struct SettingsRow {
    evaluated_at: Option<i64>,
    telemetry_at: Option<i64>,
    next_eval_at: i64,
    telemetry: Value,
}

async fn settings(st: &AppState, org: &str) -> Result<Option<SettingsRow>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT (extract(epoch FROM evaluated_at) * 1000)::bigint AS evaluated_at, (extract(epoch FROM telemetry_at) * 1000)::bigint AS telemetry_at, \
         (extract(epoch FROM next_eval_at) * 1000)::bigint AS next_eval_at, telemetry FROM scorecard_settings WHERE org_id = $1",
    )
    .bind(org)
    .fetch_optional(&st.postgres)
    .await?)
}

async fn load_scorecards(st: &AppState, org: &str) -> Result<Vec<Scorecard>, ApiError> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id::text, name, description FROM scorecards WHERE org_id = $1 ORDER BY position, lower(name)").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|(id, name, description)| Scorecard { id, name, description }).collect())
}

async fn load_rules(st: &AppState, org: &str) -> Result<Vec<ScorecardRule>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        scorecard_id: String,
        builtin: Option<String>,
        name: String,
        description: String,
        level: i16,
        scope: String,
        enabled: bool,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT r.id::text AS id, r.scorecard_id::text AS scorecard_id, r.builtin, r.name, r.description, r.level, r.scope, r.enabled FROM scorecard_rules r \
         JOIN scorecards c ON c.org_id = r.org_id AND c.id = r.scorecard_id WHERE r.org_id = $1 ORDER BY c.position, lower(c.name), r.level, lower(r.name)",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ScorecardRule { id: r.id, scorecard_id: r.scorecard_id, builtin: r.builtin, name: r.name, description: r.description, level: r.level as u8, scope: r.scope, enabled: r.enabled })
        .collect())
}

async fn load_metadata(st: &AppState, org: &str) -> Result<BTreeMap<String, Metadata>, ApiError> {
    let rows: Vec<(String, String, Value, String, String)> =
        sqlx::query_as("SELECT service, team, contacts, repo_url, docs_url FROM service_metadata WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|(service, team, contacts, repo_url, docs_url)| (service, Metadata { team, contacts: serde_json::from_value(contacts).unwrap_or_default(), repo_url, docs_url })).collect())
}

/// Whether an On-Call product's tables exist; the rule skips with this reason until it can read them.
async fn on_call_reason(st: &AppState) -> Result<String, ApiError> {
    let (exists,): (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND (table_name LIKE 'on\\_call%' OR table_name LIKE 'oncall%'))")
            .fetch_one(&st.postgres)
            .await?;
    Ok(if exists { "On-Call schedules aren't read by Scorecards yet".into() } else { "On-Call is not configured".into() })
}

async fn load_inputs(st: &AppState, org: &str, telemetry: BTreeMap<String, Telemetry>) -> Result<Inputs, ApiError> {
    let rules = load_rules(st, org).await?.into_iter().map(|r| RuleDef { id: r.id, scorecard_id: r.scorecard_id, builtin: r.builtin, level: r.level, scope: r.scope, enabled: r.enabled }).collect();
    let monitors: Vec<(i64, String, String, Vec<String>)> = sqlx::query_as("SELECT id, name, query, tags FROM monitors WHERE org_id = $1 AND NOT draft").bind(org).fetch_all(&st.postgres).await?;
    let monitor_names: HashMap<i64, String> = monitors.iter().map(|(id, name, ..)| (*id, name.clone())).collect();
    #[derive(sqlx::FromRow)]
    struct SloRow {
        name: String,
        good_query: Option<String>,
        total_query: Option<String>,
        slice_query: Option<String>,
        tags: Vec<String>,
        monitor_ids: Vec<i64>,
    }
    let slos: Vec<SloRow> = sqlx::query_as("SELECT name, good_query, total_query, slice_query, tags, monitor_ids FROM slos WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let manual_rows: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT rule_id::text, service, state, remarks FROM scorecard_manual_outcomes WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let mut manual: HashMap<String, HashMap<String, (ScorecardOutcomeState, String)>> = HashMap::new();
    for (rule, service, state, remarks) in manual_rows {
        if let Some(state) = ScorecardOutcomeState::parse(&state) {
            manual.entry(rule).or_default().insert(service, (state, remarks));
        }
    }
    Ok(Inputs {
        rules,
        telemetry,
        metadata: load_metadata(st, org).await?,
        monitors: monitors.into_iter().map(|(_, name, query, tags)| (name, query, tags)).collect(),
        slos: slos
            .into_iter()
            .map(|r| SloRef {
                name: r.name,
                queries: [r.good_query, r.total_query, r.slice_query].into_iter().flatten().collect(),
                tags: r.tags,
                monitors: r.monitor_ids.iter().filter_map(|id| monitor_names.get(id).cloned()).collect(),
            })
            .collect(),
        manual,
        org: OrgFacts { now_ms: now_ms(), on_call_teams: None, on_call_missing: on_call_reason(st).await? },
    })
}

/// Reads the org's telemetry facts: versions on entry spans (90 days), logs with trace ids
/// (7 days) and CI deployment jobs (90 days).
async fn read_telemetry(st: &AppState, org: &str) -> Result<BTreeMap<String, Telemetry>, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct VersionRow {
        service: String,
        version: String,
        first_seen: i64,
        last_seen: i64,
        recent: u64,
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct LogRow {
        service: String,
        logs: u64,
        traced: u64,
    }
    let versions = st
        .ch(org)
        .query(&format!(
            "SELECT toString(ServiceName) AS service, ResourceAttributes['service.version'] AS version, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen, \
             toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_seen, countIf(Timestamp >= now() - INTERVAL 7 DAY) AS recent \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= now() - INTERVAL {} DAY AND {ENTRY} GROUP BY service, version",
            rules::DEPLOY_WINDOW_DAYS
        ))
        .fetch_all::<VersionRow>();
    let deploys = st
        .ch(org)
        .query(&format!(
            "SELECT SpanAttributes['deployment.service.name'] AS service, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last \
             FROM otel.otel_ci_traces WHERE OrgId = {{oc_org:String}} AND SpanAttributes['deployment.service.name'] != '' AND SpanAttributes['cicd.pipeline.task.name'] != '' \
             AND Timestamp >= now() - INTERVAL {} DAY GROUP BY service",
            rules::DEPLOY_WINDOW_DAYS
        ))
        .fetch_all::<(String, i64)>();
    // Counts only, for the org's own evaluation: every log of the org (no reader's restriction).
    let now = now_ms();
    let logs_scope = scope(org, &LogAccess::org_background(), TimeRange { from: now - 7 * DAY_MS, to: now }, &[], None);
    let logs = bind(
        st.ch(org).query(&format!("SELECT service, count() AS logs, countIf(trace_id != '') AS traced FROM {} WHERE {} GROUP BY service", logs_scope.table, logs_scope.clause)),
        &logs_scope.params,
    )
    .fetch_all::<LogRow>();
    let (versions, deploys, logs) = tokio::try_join!(versions, deploys, logs)?;
    let mut out: BTreeMap<String, Telemetry> = BTreeMap::new();
    for v in versions {
        out.entry(v.service).or_default().versions.push(VersionSeen { version: v.version, first_seen: v.first_seen, last_seen: v.last_seen, recent_spans: v.recent });
    }
    for (service, last) in deploys {
        out.entry(service).or_default().last_ci_deploy = Some(last);
    }
    for l in logs.into_iter().filter(|l| !l.service.is_empty()) {
        let t = out.entry(l.service).or_default();
        t.logs = l.logs;
        t.traced_logs = l.traced;
    }
    Ok(out)
}

/// Evaluates the org's rules and replaces the stored outcomes. `refresh` reads telemetry again;
/// otherwise the last snapshot is reused (rule, outcome and metadata edits).
async fn evaluate_org(st: &AppState, org: &str, refresh: bool) -> Result<(), ApiError> {
    let Some(current) = settings(st, org).await? else { return Ok(()) };
    let refresh = refresh || current.telemetry_at.is_none();
    let telemetry = if refresh { read_telemetry(st, org).await? } else { serde_json::from_value(current.telemetry).unwrap_or_default() };
    let inputs = load_inputs(st, org, telemetry).await?;
    let outcomes = evaluate_all(&inputs);

    let mut tx = st.postgres.begin().await?;
    sqlx::query("DELETE FROM scorecard_outcomes WHERE org_id = $1").bind(org).execute(&mut *tx).await?;
    let rows = serde_json::to_value(outcomes.iter().map(|o| serde_json::json!({"rule": o.rule_id, "service": o.service, "state": o.state.as_str(), "reason": o.reason})).collect::<Vec<_>>())
        .map_err(anyhow::Error::from)?;
    sqlx::query(
        "INSERT INTO scorecard_outcomes (org_id, rule_id, service, state, reason) \
         SELECT $1, (e->>'rule')::uuid, e->>'service', e->>'state', e->>'reason' FROM jsonb_array_elements($2::jsonb) AS e",
    )
    .bind(org)
    .bind(rows)
    .execute(&mut *tx)
    .await?;
    // Reports → Scorecards Performance: today's level per service (the day's last evaluation wins).
    let scorecards = load_scorecards(st, org).await?;
    let levels = serde_json::to_value(score_services(&inputs, &scorecards, &outcomes).iter().map(|s| serde_json::json!({"service": s.service, "team": s.team, "level": s.level})).collect::<Vec<_>>())
        .map_err(anyhow::Error::from)?;
    sqlx::query("DELETE FROM portal_scorecard_levels WHERE org_id = $1 AND day = current_date").bind(org).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO portal_scorecard_levels (org_id, day, service, team, level) \
         SELECT $1, current_date, e->>'service', e->>'team', (e->>'level')::smallint FROM jsonb_array_elements($2::jsonb) AS e",
    )
    .bind(org)
    .bind(levels)
    .execute(&mut *tx)
    .await?;
    if refresh {
        let snapshot = serde_json::to_value(&inputs.telemetry).map_err(anyhow::Error::from)?;
        sqlx::query(
            "UPDATE scorecard_settings SET evaluated_at = now(), telemetry = $2, telemetry_at = now(), next_eval_at = now() + make_interval(hours => $3), claimed_until = NULL WHERE org_id = $1",
        )
        .bind(org)
        .bind(snapshot)
        .bind(EVALUATION_INTERVAL_HOURS)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query("UPDATE scorecard_settings SET evaluated_at = now() WHERE org_id = $1").bind(org).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn overview(st: &AppState, org: &str) -> Result<ScorecardsOverview, ApiError> {
    let recommended = recommendations();
    let Some(s) = settings(st, org).await? else {
        return Ok(ScorecardsOverview {
            set_up: false,
            evaluated_at: None,
            telemetry_at: None,
            next_evaluation_at: None,
            scorecards: vec![],
            rules: vec![],
            outcomes: vec![],
            services: vec![],
            recommended,
        });
    };
    let scorecards = load_scorecards(st, org).await?;
    let api_rules = load_rules(st, org).await?;
    let outcome_rows: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT rule_id::text, service, state, reason FROM scorecard_outcomes WHERE org_id = $1 ORDER BY service").bind(org).fetch_all(&st.postgres).await?;
    let outcomes: Vec<ScorecardOutcome> =
        outcome_rows.into_iter().filter_map(|(rule_id, service, state, reason)| ScorecardOutcomeState::parse(&state).map(|state| ScorecardOutcome { rule_id, service, state, reason })).collect();
    let inputs = Inputs {
        rules: api_rules
            .iter()
            .map(|r| RuleDef { id: r.id.clone(), scorecard_id: r.scorecard_id.clone(), builtin: r.builtin.clone(), level: r.level, scope: r.scope.clone(), enabled: r.enabled })
            .collect(),
        telemetry: serde_json::from_value(s.telemetry).unwrap_or_default(),
        metadata: load_metadata(st, org).await?,
        ..Default::default()
    };
    let services = score_services(&inputs, &scorecards, &outcomes);
    Ok(ScorecardsOverview {
        set_up: true,
        evaluated_at: s.evaluated_at,
        telemetry_at: s.telemetry_at,
        next_evaluation_at: Some(s.next_eval_at),
        scorecards,
        rules: api_rules,
        outcomes,
        services,
        recommended,
    })
}

async fn require_set_up(st: &AppState, org: &str) -> Result<(), ApiError> {
    if settings(st, org).await?.is_none() {
        return Err(ApiError::BadRequest("Set up scorecards first".into()));
    }
    Ok(())
}

/// Finds a scorecard by name or creates it.
async fn scorecard_id(tx: &mut sqlx::PgConnection, org: &str, name: &str, description: &str) -> Result<String, ApiError> {
    if let Some((id,)) = sqlx::query_as::<_, (String,)>("SELECT id::text FROM scorecards WHERE org_id = $1 AND lower(name) = lower($2)").bind(org).bind(name).fetch_optional(&mut *tx).await? {
        return Ok(id);
    }
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM scorecards WHERE org_id = $1").bind(org).fetch_one(&mut *tx).await?;
    if count >= MAX_SCORECARDS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_SCORECARDS} scorecards")));
    }
    let (id,): (String,) = sqlx::query_as("INSERT INTO scorecards (org_id, name, description, position) VALUES ($1, $2, $3, $4) RETURNING id::text")
        .bind(org)
        .bind(name)
        .bind(description)
        .bind(count as i32)
        .fetch_one(&mut *tx)
        .await?;
    Ok(id)
}

struct RuleRow {
    builtin: Option<String>,
    name: String,
}

async fn rule_row(st: &AppState, org: &str, id: &str) -> Result<RuleRow, ApiError> {
    let missing = || ApiError::NotFound("No such scorecard rule".into());
    if !valid_uuid(id) {
        return Err(missing());
    }
    let row: Option<(Option<String>, String)> = sqlx::query_as("SELECT builtin, name FROM scorecard_rules WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).fetch_optional(&st.postgres).await?;
    let (builtin, name) = row.ok_or_else(missing)?;
    Ok(RuleRow { builtin, name })
}

/// Checks and stores a service's metadata, then evaluates from the last snapshot (also used by the
/// Software Catalog's service definitions). Answers the stored service name and metadata.
pub(crate) async fn save_metadata(st: &AppState, ctx: &AuthContext, service: &str, input: &ServiceMetadataInput) -> Result<(String, ServiceMetadataInput), ApiError> {
    let service = check_service(service).map_err(ApiError::BadRequest)?;
    let meta = check_metadata(input).map_err(ApiError::BadRequest)?;
    let contacts = serde_json::to_value(&meta.contacts).map_err(anyhow::Error::from)?;
    sqlx::query(
        "INSERT INTO service_metadata (org_id, service, team, contacts, repo_url, docs_url, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7::uuid) \
         ON CONFLICT (org_id, service) DO UPDATE SET team = EXCLUDED.team, contacts = EXCLUDED.contacts, repo_url = EXCLUDED.repo_url, docs_url = EXCLUDED.docs_url, \
         updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&service)
    .bind(&meta.team)
    .bind(contacts)
    .bind(&meta.repo_url)
    .bind(&meta.docs_url)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    evaluate_org(st, &ctx.org_id, false).await?;
    Ok((service, ServiceMetadataInput { team: meta.team, contacts: meta.contacts, repo_url: meta.repo_url, docs_url: meta.docs_url }))
}

// ================================================================ scheduler

/// Claims up to `limit` orgs due for evaluation and holds them while they run.
async fn claim(st: &AppState, limit: i64) -> Result<Vec<String>, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let orgs: Vec<(String,)> = sqlx::query_as(
        "SELECT s.org_id FROM scorecard_settings s JOIN orgs o ON o.id = s.org_id WHERE s.next_eval_at <= now() AND (s.claimed_until IS NULL OR s.claimed_until < now()) \
         AND o.status = 'active' ORDER BY s.next_eval_at LIMIT $1 FOR UPDATE OF s SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for (org,) in &orgs {
        sqlx::query("UPDATE scorecard_settings SET claimed_until = now() + make_interval(secs => $2), next_eval_at = now() + make_interval(hours => $3) WHERE org_id = $1")
            .bind(org)
            .bind(CLAIM_SECONDS)
            .bind(EVALUATION_INTERVAL_HOURS)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(orgs.into_iter().map(|(o,)| o).collect())
}

/// Evaluates every set-up org hourly (several servers can run it: orgs are claimed with SKIP LOCKED).
pub async fn run_scheduler(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = match claim(&st, 10).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(error = %e, "claiming scorecard evaluations failed");
                continue;
            }
        };
        for org in due {
            if let Err(e) = evaluate_org(&st, &org, true).await {
                tracing::warn!(org = %org, error = %e, "scorecard evaluation failed");
                let _ = sqlx::query("UPDATE scorecard_settings SET claimed_until = NULL WHERE org_id = $1").bind(&org).execute(&st.postgres).await;
            }
        }
    }
}

// ================================================================ handlers

/// Scorecards, rules, the last outcomes and every service's levels; `setUp` is false before setup.
#[utoipa::path(get, path = "/api/v1/scorecards", operation_id = "get_scorecards", responses((status = 200, body = ScorecardsOverview)), tag = "scorecards")]
pub async fn get_overview(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<ScorecardsOverview> {
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

/// Creates the recommended scorecards with the chosen built-in rules enabled, and evaluates them.
#[utoipa::path(post, path = "/api/v1/scorecards/setup", operation_id = "setup_scorecards", request_body = ScorecardSetupInput,
    responses((status = 200, body = ScorecardsOverview), (status = 400, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn setup(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<ScorecardSetupInput>) -> ApiResult<ScorecardsOverview> {
    let chosen: HashSet<BuiltinRule> = input.rules.iter().map(|k| BuiltinRule::from_key(k).ok_or_else(|| ApiError::BadRequest(format!("Unknown rule {k}")))).collect::<Result<_, _>>()?;
    let mut tx = st.postgres.begin().await?;
    sqlx::query("INSERT INTO scorecard_settings (org_id) VALUES ($1) ON CONFLICT (org_id) DO NOTHING").bind(&ctx.org_id).execute(&mut *tx).await?;
    for (name, description) in rules::BUILTIN_SCORECARDS {
        let card = scorecard_id(&mut tx, &ctx.org_id, name, description).await?;
        for rule in BuiltinRule::ALL.iter().filter(|r| r.scorecard() == name) {
            sqlx::query(
                "INSERT INTO scorecard_rules (org_id, scorecard_id, builtin, name, description, level, enabled, created_by) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8::uuid) \
                 ON CONFLICT (org_id, builtin) DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = now()",
            )
            .bind(&ctx.org_id)
            .bind(&card)
            .bind(rule.key())
            .bind(rule.name())
            .bind(rule.description())
            .bind(i16::from(rule.level()))
            .bind(chosen.contains(rule))
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    evaluate_org(&st, &ctx.org_id, true).await?;
    ctx.audit(&st, "scorecards.setup", "scorecards", &format!("{} built-in rules enabled", chosen.len())).await;
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

/// Reads telemetry again and evaluates every rule now.
#[utoipa::path(post, path = "/api/v1/scorecards/evaluate", operation_id = "evaluate_scorecards",
    responses((status = 200, body = ScorecardsOverview), (status = 400, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn evaluate_now(State(st): State<AppState>, Writer(ctx): Writer) -> ApiResult<ScorecardsOverview> {
    require_set_up(&st, &ctx.org_id).await?;
    evaluate_org(&st, &ctx.org_id, true).await?;
    ctx.audit(&st, "scorecards.evaluated", "scorecards", "evaluated on demand").await;
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

/// Creates a custom rule (its outcomes come from `POST /api/v1/scorecards/rules/{id}/outcomes`).
#[utoipa::path(post, path = "/api/v1/scorecards/rules", operation_id = "create_scorecard_rule", request_body = ScorecardRuleInput,
    responses((status = 200, body = ScorecardsOverview), (status = 400, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn create_rule(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<ScorecardRuleInput>) -> ApiResult<ScorecardsOverview> {
    require_set_up(&st, &ctx.org_id).await?;
    let bad = ApiError::BadRequest;
    let name = check_text(&input.name, "rule name", MAX_NAME_CHARS, true).map_err(bad)?;
    let card_name = check_text(&input.scorecard, "scorecard name", MAX_SCORECARD_NAME_CHARS, true).map_err(bad)?;
    let description = check_text(&input.description, "description", MAX_DESCRIPTION_CHARS, false).map_err(bad)?;
    let scope = check_text(&input.scope, "scope", MAX_SCOPE_CHARS, false).map_err(bad)?;
    let level = check_level(input.level).map_err(bad)?;
    let mut tx = st.postgres.begin().await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM scorecard_rules WHERE org_id = $1 AND builtin IS NULL").bind(&ctx.org_id).fetch_one(&mut *tx).await?;
    if count >= MAX_CUSTOM_RULES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_CUSTOM_RULES} custom rules")));
    }
    let card = scorecard_id(&mut tx, &ctx.org_id, &card_name, "").await?;
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO scorecard_rules (org_id, scorecard_id, name, description, level, scope, created_by) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7::uuid) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&card)
            .bind(&name)
            .bind(&description)
            .bind(i16::from(level))
            .bind(&scope)
            .bind(&ctx.user_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    evaluate_org(&st, &ctx.org_id, false).await?;
    ctx.audit(&st, "scorecards.rule_created", &id, &format!("{name} (level {level}, {card_name})")).await;
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

/// Changes a rule: level, scope and enabled for built-in rules, also name and description for custom ones.
#[utoipa::path(put, path = "/api/v1/scorecards/rules/{id}", operation_id = "update_scorecard_rule", params(("id" = String, Path)), request_body = ScorecardRuleUpdate,
    responses((status = 200, body = ScorecardsOverview), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn update_rule(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<ScorecardRuleUpdate>) -> ApiResult<ScorecardsOverview> {
    let row = rule_row(&st, &ctx.org_id, &id).await?;
    let bad = ApiError::BadRequest;
    if row.builtin.is_some() && (input.name.is_some() || input.description.is_some()) {
        return Err(bad("Built-in rules keep their name and description".into()));
    }
    let name = input.name.as_deref().map(|n| check_text(n, "rule name", MAX_NAME_CHARS, true)).transpose().map_err(bad)?;
    let description = input.description.as_deref().map(|d| check_text(d, "description", MAX_DESCRIPTION_CHARS, false)).transpose().map_err(bad)?;
    let scope = input.scope.as_deref().map(|s| check_text(s, "scope", MAX_SCOPE_CHARS, false)).transpose().map_err(bad)?;
    let level = input.level.map(check_level).transpose().map_err(bad)?;
    sqlx::query(
        "UPDATE scorecard_rules SET name = coalesce($3, name), description = coalesce($4, description), level = coalesce($5, level), scope = coalesce($6, scope), \
         enabled = coalesce($7, enabled), updated_at = now() WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&name)
    .bind(&description)
    .bind(level.map(i16::from))
    .bind(&scope)
    .bind(input.enabled)
    .execute(&st.postgres)
    .await?;
    evaluate_org(&st, &ctx.org_id, false).await?;
    let mut changes = Vec::new();
    if let Some(l) = level {
        changes.push(format!("level {l}"));
    }
    if let Some(s) = &scope {
        changes.push(format!("scope '{s}'"));
    }
    if let Some(e) = input.enabled {
        changes.push(if e { "enabled".into() } else { "disabled".into() });
    }
    if name.is_some() || description.is_some() {
        changes.push("renamed or described".into());
    }
    ctx.audit(&st, "scorecards.rule_updated", &id, &format!("{}: {}", name.as_deref().unwrap_or(&row.name), changes.join(", "))).await;
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

/// Deletes a custom rule and its outcomes (built-in rules are disabled instead).
#[utoipa::path(delete, path = "/api/v1/scorecards/rules/{id}", operation_id = "delete_scorecard_rule", params(("id" = String, Path)),
    responses((status = 204), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn delete_rule(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row = rule_row(&st, &ctx.org_id, &id).await?;
    if row.builtin.is_some() {
        return Err(ApiError::BadRequest("Built-in rules can't be deleted; disable them instead".into()));
    }
    sqlx::query("DELETE FROM scorecard_rules WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    evaluate_org(&st, &ctx.org_id, false).await?;
    ctx.audit(&st, "scorecards.rule_deleted", &id, &row.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Reports outcomes of a custom rule per service (pass, fail or skip, with remarks); replaces
/// earlier outcomes of the same services.
#[utoipa::path(post, path = "/api/v1/scorecards/rules/{id}/outcomes", operation_id = "set_scorecard_outcomes", params(("id" = String, Path)), request_body = ScorecardOutcomesInput,
    responses((status = 200, body = ScorecardsOverview), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn set_outcomes(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<ScorecardOutcomesInput>) -> ApiResult<ScorecardsOverview> {
    let row = rule_row(&st, &ctx.org_id, &id).await?;
    if row.builtin.is_some() {
        return Err(ApiError::BadRequest("Built-in rules are evaluated from the organization's data; report outcomes for custom rules".into()));
    }
    if input.outcomes.is_empty() || input.outcomes.len() > MAX_OUTCOMES {
        return Err(ApiError::BadRequest(format!("Send 1 to {MAX_OUTCOMES} outcomes")));
    }
    let mut rows = Vec::new();
    for o in &input.outcomes {
        let service = check_service(&o.service).map_err(ApiError::BadRequest)?;
        let remarks = check_text(&o.remarks, "remarks", MAX_REMARKS_CHARS, false).map_err(ApiError::BadRequest)?;
        rows.push(serde_json::json!({"service": service, "state": o.state.as_str(), "remarks": remarks}));
    }
    sqlx::query(
        "INSERT INTO scorecard_manual_outcomes (org_id, rule_id, service, state, remarks, updated_by) \
         SELECT DISTINCT ON (e->>'service') $1, $2::uuid, e->>'service', e->>'state', e->>'remarks', $3::uuid FROM jsonb_array_elements($4::jsonb) WITH ORDINALITY AS x(e, ord) ORDER BY e->>'service', ord DESC \
         ON CONFLICT (rule_id, service) DO UPDATE SET state = EXCLUDED.state, remarks = EXCLUDED.remarks, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&ctx.user_id)
    .bind(Value::Array(rows))
    .execute(&st.postgres)
    .await?;
    evaluate_org(&st, &ctx.org_id, false).await?;
    ctx.audit(&st, "scorecards.outcomes_reported", &id, &format!("{}: {} outcomes", row.name, input.outcomes.len())).await;
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

/// Sets a service's team, contacts, code repository and docs (rules read them on the next evaluation, which runs at once).
#[utoipa::path(put, path = "/api/v1/scorecards/services/{service}/metadata", operation_id = "update_service_metadata", params(("service" = String, Path)), request_body = ServiceMetadataInput,
    responses((status = 200, body = ScorecardsOverview), (status = 400, body = crate::error::ErrorBody)), tag = "scorecards")]
pub async fn update_metadata(State(st): State<AppState>, Writer(ctx): Writer, Path(service): Path<String>, Json(input): Json<ServiceMetadataInput>) -> ApiResult<ScorecardsOverview> {
    let (service, meta) = save_metadata(&st, &ctx, &service, &input).await?;
    ctx.audit(&st, "scorecards.service_metadata_updated", &service, &format!("team '{}', {} contacts", meta.team, meta.contacts.len())).await;
    Ok(Json(overview(&st, &ctx.org_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ScorecardOutcomeState::{Fail, Pass, Skip};

    const NOW: i64 = 1_800_000_000_000;

    fn rule(id: &str, card: &str, builtin: Option<BuiltinRule>, level: u8, scope: &str) -> RuleDef {
        RuleDef { id: id.into(), scorecard_id: card.into(), builtin: builtin.map(|b| b.key().into()), level, scope: scope.into(), enabled: true }
    }

    fn spans(recent: u64) -> Telemetry {
        Telemetry { versions: vec![VersionSeen { version: "1.4.0".into(), first_seen: NOW - 3 * DAY_MS, last_seen: NOW, recent_spans: recent }], ..Default::default() }
    }

    fn inputs() -> Inputs {
        let mut telemetry = BTreeMap::new();
        telemetry.insert("order-service".to_string(), spans(100));
        telemetry.insert("billing".to_string(), spans(40));
        // Seen only long ago: not listed unless it has metadata.
        telemetry.insert("legacy-batch".to_string(), spans(0));
        let mut metadata = BTreeMap::new();
        metadata.insert("order-service".to_string(), Metadata { team: "checkout".into(), ..Default::default() });
        metadata.insert("catalog-service".to_string(), Metadata { team: "catalog".into(), ..Default::default() });
        Inputs {
            rules: vec![
                rule("r-team", "c-own", Some(BuiltinRule::TeamDefined), 1, ""),
                rule("r-monitors", "c-prod", Some(BuiltinRule::MonitorsDefined), 1, ""),
                rule("r-slos", "c-prod", Some(BuiltinRule::SlosDefined), 2, "team:checkout"),
                rule("r-custom", "c-prod", None, 2, "-service:catalog-service"),
            ],
            telemetry,
            metadata,
            monitors: vec![
                ("[Checkout] error rate".into(), "avg:trace.error_rate{service:order-service}".into(), vec![]),
                ("Billing errors".into(), "logs(\"status:error\")".into(), vec!["service:billing".into()]),
            ],
            slos: vec![SloRef { name: "Checkout availability".into(), monitors: vec!["[Checkout] error rate".into()], ..Default::default() }],
            manual: HashMap::from([("r-custom".to_string(), HashMap::from([("order-service".to_string(), (Fail, "No load test this quarter".to_string()))]))]),
            org: OrgFacts { now_ms: NOW, ..Default::default() },
        }
    }

    fn outcome<'a>(all: &'a [ScorecardOutcome], rule: &str, service: &str) -> Option<&'a ScorecardOutcome> {
        all.iter().find(|o| o.rule_id == rule && o.service == service)
    }

    #[test]
    fn services_are_instrumented_ones_and_those_with_metadata() {
        assert_eq!(service_names(&inputs()), vec!["billing", "catalog-service", "order-service"]);
    }

    #[test]
    fn evaluation_covers_enabled_rules_within_their_scope() {
        let mut i = inputs();
        let all = evaluate_all(&i);
        assert_eq!(outcome(&all, "r-team", "order-service").unwrap().state, Pass);
        assert_eq!(outcome(&all, "r-team", "billing").unwrap().state, Fail);
        assert_eq!(outcome(&all, "r-monitors", "billing").unwrap().state, Pass);
        assert_eq!(outcome(&all, "r-monitors", "catalog-service").unwrap().state, Fail);
        // SLO scoped to team:checkout; it references the service through its monitor.
        let slo = outcome(&all, "r-slos", "order-service").unwrap();
        assert_eq!(slo.state, Pass, "{}", slo.reason);
        assert!(outcome(&all, "r-slos", "billing").is_none());
        // Custom rule: reported outcome, otherwise skipped; out of scope: none.
        let custom = outcome(&all, "r-custom", "order-service").unwrap();
        assert_eq!((custom.state, custom.reason.as_str()), (Fail, "No load test this quarter"));
        assert_eq!(outcome(&all, "r-custom", "billing").unwrap().state, Skip);
        assert!(outcome(&all, "r-custom", "catalog-service").is_none());

        i.rules[0].enabled = false;
        assert!(evaluate_all(&i).iter().all(|o| o.rule_id != "r-team"));
    }

    #[test]
    fn services_get_levels_overall_and_per_scorecard() {
        let i = inputs();
        let cards =
            vec![Scorecard { id: "c-own".into(), name: "Ownership".into(), description: String::new() }, Scorecard { id: "c-prod".into(), name: "Production".into(), description: String::new() }];
        let all = evaluate_all(&i);
        let services = score_services(&i, &cards, &all);
        let order = services.iter().find(|s| s.service == "order-service").unwrap();
        // L1 team + monitors pass, L2 SLO passes but the custom L2 rule fails.
        assert_eq!((order.level, order.pass, order.fail, order.skip), (1, 3, 1, 0));
        assert_eq!(order.levels["c-own"], 3);
        assert_eq!(order.levels["c-prod"], 1);
        assert_eq!(order.score, Some(75.0));
        assert_eq!(order.team, "checkout");
        assert!(order.instrumented);
        let billing = services.iter().find(|s| s.service == "billing").unwrap();
        assert_eq!(billing.level, 0);
        let catalog = services.iter().find(|s| s.service == "catalog-service").unwrap();
        assert!(!catalog.instrumented);
    }

    #[test]
    fn metadata_input_is_checked() {
        let ok = ServiceMetadataInput {
            team: " @checkout ".into(),
            contacts: vec![
                ServiceContact { r#type: "email".into(), name: "On-call".into(), value: "checkout-oncall@example.com".into() },
                ServiceContact { r#type: "slack".into(), name: String::new(), value: "#checkout".into() },
                ServiceContact { r#type: "link".into(), name: String::new(), value: "  ".into() },
            ],
            repo_url: "https://git.example.com/example-org/order-service".into(),
            docs_url: String::new(),
        };
        let m = check_metadata(&ok).unwrap();
        assert_eq!(m.team, "checkout");
        assert_eq!(m.contacts.len(), 2);
        let bad_url = ServiceMetadataInput { repo_url: "javascript:alert(1)".into(), ..Default::default() };
        assert!(check_metadata(&bad_url).is_err());
        let bad_link = ServiceMetadataInput { contacts: vec![ServiceContact { r#type: "link".into(), name: String::new(), value: "data:text/html,x".into() }], ..Default::default() };
        assert!(check_metadata(&bad_link).is_err());
        let bad_email = ServiceMetadataInput { contacts: vec![ServiceContact { r#type: "email".into(), name: String::new(), value: "nobody".into() }], ..Default::default() };
        assert!(check_metadata(&bad_email).is_err());
        let bad_type = ServiceMetadataInput { contacts: vec![ServiceContact { r#type: "pager".into(), name: String::new(), value: "x".into() }], ..Default::default() };
        assert!(check_metadata(&bad_type).is_err());
    }

    #[test]
    fn inputs_are_bounded() {
        assert!(check_level(0).is_err() && check_level(4).is_err() && check_level(2).is_ok());
        assert!(check_service("").is_err());
        assert!(check_service(&"x".repeat(201)).is_err());
        assert_eq!(check_service(" billing ").unwrap(), "billing");
        assert!(check_text("  ", "rule name", 10, true).is_err());
        assert!(valid_uuid("0b9e4c7a-1f2d-4c3b-9a8e-7d6c5b4a3f21"));
        assert!(!valid_uuid("1; DROP TABLE x"));
    }

    #[test]
    fn telemetry_snapshots_round_trip() {
        let t = BTreeMap::from([("billing".to_string(), Telemetry { logs: 5, traced_logs: 2, last_ci_deploy: Some(NOW), ..spans(3) })]);
        let back: BTreeMap<String, Telemetry> = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        assert_eq!(serde_json::from_value::<BTreeMap<String, Telemetry>>(serde_json::json!({})).unwrap(), BTreeMap::new());
    }
}
