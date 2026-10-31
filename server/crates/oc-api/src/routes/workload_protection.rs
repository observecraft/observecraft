//! Workload Protection's pages (`/api/v1/workload-protection/...`): the summary, the agent events
//! explorer, findings, the Agents inventory and the policies and agent rules.
//!
//! - Agent events are the logs the Datadog Agent's runtime security module sends
//!   (`source:runtime-security-agent`, the event as JSON: `@agent.rule_id`, `@evt.category`,
//!   `@process.*`, `@file.*`, `@container.id`), read with the user's log access.
//! - Monitored events (everything the Agent's probes saw, not only rule matches) come from its
//!   `datadog.runtime_security.perf_buffer.events.write` counter by `event_type`.
//! - Findings are the `workload_activity` finding rules (`workload_catalog.rs`) evaluated over the
//!   events: a host or container fails a rule when it matched one of its agent rules in the range.
//!   Muting uses Code Security's triage (`workload-finding:<rule>:<resource>`).
//! - Policies and agent rules are the records of the Datadog API (`datadog_api/csm_threats.rs`);
//!   detection rules are `security_rules` of type `workload_security` (Cloud SIEM's pages and API).

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use oc_query::{parse_query, LogsSchema, Param, Schema, Term};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::{IntoParams, ToSchema};

use super::datadog_api::csm_threats::{self as cws, CsmAgent, PolicyRow, RuleRow};
use super::logs::{bind, scope, TimeRange};
use super::metrics::{points_subquery, with_window};
use super::workload_catalog::{self as catalog, AGENT_RULES, FINDING_RULES, PACKS, POLICIES, TACTICS};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::AppState;

/// The `source` of the Agent's runtime security events.
pub const SOURCE: &str = "runtime-security-agent";
/// The Agent's counter of events its probes wrote, by `event_type`.
pub const MONITORED_METRIC: &str = "datadog.runtime_security.perf_buffer.events.write";
const DAY: i64 = 86_400_000;

fn api_err((status, Json(body)): (StatusCode, Json<Value>)) -> ApiError {
    let msg = body["errors"].get(0).map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e["title"].as_str().unwrap_or("error").to_string())).unwrap_or_default();
    match status {
        StatusCode::NOT_FOUND => ApiError::NotFound(msg),
        StatusCode::CONFLICT => ApiError::Conflict(msg),
        StatusCode::FORBIDDEN => ApiError::Forbidden(msg),
        s if s.is_client_error() => ApiError::BadRequest(msg),
        _ => ApiError::Internal(anyhow::anyhow!(msg)),
    }
}

#[derive(Deserialize, IntoParams, Default)]
#[into_params(parameter_in = Query)]
pub struct WpRangeQuery {
    /// Epoch ms; defaults to 7 days before `to`.
    pub from: Option<i64>,
    /// Epoch ms; defaults to now.
    pub to: Option<i64>,
}

impl WpRangeQuery {
    fn range(&self) -> Result<TimeRange, ApiError> {
        let to = self.to.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let r = TimeRange { from: self.from.unwrap_or(to - 7 * DAY), to };
        r.validate()?;
        Ok(r)
    }
}

/// The event type the Agent's counter reports, as the event categories.
pub fn event_type_category(t: &str) -> &'static str {
    match t {
        "exec" | "fork" | "exit" | "signal" | "ptrace" | "setuid" | "setgid" | "capset" | "selinux" => "process",
        "open" | "chmod" | "chown" | "unlink" | "rename" | "mkdir" | "rmdir" | "link" | "utimes" | "setxattr" | "removexattr" | "mount" | "umount" | "splice" | "chdir" => "file",
        "dns" | "bind" | "connect" | "accept" | "imds" | "net_device" | "veth_pair" | "packet" | "network_flow_monitor" => "network",
        _ => "kernel",
    }
}

/// `File Activity` → `file`.
fn category_key(c: &str) -> &'static str {
    match c {
        "File Activity" => "file",
        "Process Activity" => "process",
        "Network Activity" => "network",
        _ => "kernel",
    }
}

fn source_terms(query: &str) -> Vec<Term> {
    let mut terms = parse_query(query);
    terms.push(Term::Tag { key: "source".into(), values: vec![SOURCE.into()], negate: false });
    terms
}

#[derive(clickhouse::Row, Deserialize)]
struct KeyCount {
    k: String,
    n: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct BucketCount {
    b: i64,
    k: String,
    n: u64,
}

/// Bucket size for about 60 buckets over the range, in whole minutes.
fn bucket_ms(r: TimeRange) -> i64 {
    (((r.to - r.from) / 60).max(60_000) / 60_000) * 60_000
}

/// Event counts per (bucket, value of `field`) over the agent events matching `query`.
async fn event_series(st: &AppState, org: &str, access: &LogAccess, query: &str, field: &str, r: TimeRange) -> Result<WpSeries, ApiError> {
    let s = scope(org, access, r, &source_terms(query), None);
    let (e, p) = LogsSchema.field(field);
    let step = bucket_ms(r);
    let mut params = vec![Param::Int(step), Param::Int(step)];
    params.extend(p);
    params.extend(s.params);
    let rows: Vec<BucketCount> = bind(
        st.ch(org).query(&format!(
            "SELECT toInt64(intDiv(toUnixTimestamp64Milli(timestamp), ?) * ?) AS b, toString({e}) AS k, count() AS n FROM {} WHERE {} GROUP BY b, k ORDER BY b",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all()
    .await?;
    Ok(WpSeries::from_rows(r, step, rows))
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct WpSeries {
    /// Bucket starts, epoch ms.
    pub times: Vec<i64>,
    pub groups: Vec<WpSeriesGroup>,
}

#[derive(Serialize, ToSchema)]
pub struct WpSeriesGroup {
    pub name: String,
    pub values: Vec<u64>,
}

impl WpSeries {
    fn from_rows(r: TimeRange, step: i64, rows: Vec<BucketCount>) -> WpSeries {
        let start = r.from / step * step;
        let times: Vec<i64> = (0..).map(|i| start + i * step).take_while(|t| *t <= r.to).collect();
        let mut groups: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for row in rows {
            let i = ((row.b - start) / step) as usize;
            if i < times.len() {
                groups.entry(if row.k.is_empty() { "other".into() } else { row.k }).or_insert_with(|| vec![0; times.len()])[i] += row.n;
            }
        }
        WpSeries { times, groups: groups.into_iter().map(|(name, values)| WpSeriesGroup { name, values }).collect() }
    }
}

async fn counts_by(st: &AppState, org: &str, access: &LogAccess, query: &str, field: &str, r: TimeRange, limit: usize) -> Result<Vec<KeyCount>, ApiError> {
    let s = scope(org, access, r, &source_terms(query), None);
    let (e, p) = LogsSchema.field(field);
    let mut params: Vec<Param> = p.into_iter().collect();
    params.extend(s.params);
    Ok(bind(st.ch(org).query(&format!("SELECT toString({e}) AS k, count() AS n FROM {} WHERE {} GROUP BY k ORDER BY n DESC LIMIT {limit}", s.table, s.clause)), &params).fetch_all().await?)
}

/// Monitored events per category and per bucket, from the Agents' counter.
async fn monitored(st: &AppState, org: &str, r: TimeRange) -> Result<(BTreeMap<String, u64>, WpSeries), ApiError> {
    let step = bucket_ms(r);
    let src = points_subquery("sum", &[MONITORED_METRIC]);
    let rows: Vec<BucketCount> = with_window(st.ch(org), r.from, r.to)
        .query(&format!(
            "SELECT toInt64(intDiv(toUnixTimestamp64Milli(TimeUnix), ?) * ?) AS b, Attributes['event_type'] AS k, toUInt64(sum(Value)) AS n FROM {src} \
             WHERE OrgId = {{oc_org:String}} AND MetricName = ? GROUP BY b, k"
        ))
        .bind(step)
        .bind(step)
        .bind(MONITORED_METRIC)
        .fetch_all()
        .await?;
    let mut totals: BTreeMap<String, u64> = ["file", "process", "network", "kernel"].iter().map(|k| (k.to_string(), 0)).collect();
    let rows: Vec<BucketCount> = rows
        .into_iter()
        .map(|mut row| {
            let c = event_type_category(&row.k);
            *totals.entry(c.into()).or_default() += row.n;
            row.k = c.into();
            row
        })
        .collect();
    Ok((totals, WpSeries::from_rows(r, step, rows)))
}

// ================================================================ summary

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpSummary {
    pub agents: WpAgentCounts,
    /// Events the Agents' probes saw, per category (file, process, network, kernel).
    pub monitored: BTreeMap<String, u64>,
    pub monitored_series: WpSeries,
    /// Events that matched an agent rule and were sent, per category.
    pub captured: BTreeMap<String, u64>,
    pub captured_series: WpSeries,
    pub policies: usize,
    pub agent_rules: usize,
    pub detection_rules: usize,
    pub finding_rules: usize,
    /// Signals of `workload_security` rules raised in the range, by severity.
    pub signals: BTreeMap<String, u64>,
    pub signal_series: WpSeries,
    /// Archived signals by archive reason, and the open and under review ones.
    pub triage: Vec<WpCount>,
    /// Failing findings by severity.
    pub findings: BTreeMap<String, u64>,
    pub tactics: Vec<WpTactic>,
    pub packs: Vec<WpPack>,
    pub agent_policies: Vec<WpPolicyCard>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpAgentCounts {
    pub hosts: usize,
    pub hosts_enabled: usize,
    pub serverless: usize,
    pub serverless_enabled: usize,
}

#[derive(Serialize, ToSchema)]
pub struct WpCount {
    pub name: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpTactic {
    pub id: String,
    pub name: String,
    /// Enabled agent rules with this tactic.
    pub rules: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpPack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rules: usize,
    pub enabled_rules: usize,
    /// active (some rules enabled) | available
    pub status: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpPolicyCard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rules: usize,
}

#[derive(sqlx::FromRow)]
struct SignalRow {
    severity: String,
    state: String,
    archive_reason: String,
    day: i64,
}

#[utoipa::path(get, path = "/api/v1/workload-protection/summary", operation_id = "workload_protection_summary", params(WpRangeQuery), responses((status = 200, body = WpSummary)), tag = "workload-protection")]
pub async fn summary(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Query(q): Query<WpRangeQuery>) -> ApiResult<WpSummary> {
    let org = ctx.org_id.as_str();
    let r = q.range()?;
    let agents = cws::agents(&st, org).await?;
    let (hosts, serverless): (Vec<&CsmAgent>, Vec<&CsmAgent>) = agents.iter().partition(|a| a.aws_fargate.is_empty());
    let policies = cws::policies(&st, org).await?;
    let rules = cws::rules(&st, org, None).await?;
    let (monitored, monitored_series) = monitored(&st, org, r).await?;
    let mut captured: BTreeMap<String, u64> = ["file", "process", "network", "kernel"].iter().map(|k| (k.to_string(), 0)).collect();
    for c in counts_by(&st, org, &access, "", "@evt.category", r, 20).await? {
        *captured.entry(category_key(&c.k).into()).or_default() += c.n;
    }
    let mut captured_series = event_series(&st, org, &access, "", "@evt.category", r).await?;
    for g in &mut captured_series.groups {
        g.name = category_key(&g.name).into();
    }
    super::security_rules::ensure_defaults(&st, org).await?;
    let detection_rules: i64 = sqlx::query_scalar("SELECT count(*) FROM security_rules WHERE org_id = $1 AND type = 'workload_security' AND is_enabled").bind(org).fetch_one(&st.postgres).await?;
    let signal_rows: Vec<SignalRow> = sqlx::query_as(
        "SELECT severity, state, archive_reason, (extract(epoch FROM date_trunc('day', first_seen)) * 1000)::bigint AS day FROM security_signals \
         WHERE org_id = $1 AND rule_type = 'workload_security' AND last_seen >= to_timestamp($2 / 1000.0) AND first_seen <= to_timestamp($3 / 1000.0)",
    )
    .bind(org)
    .bind(r.from)
    .bind(r.to)
    .fetch_all(&st.postgres)
    .await?;
    let mut signals: BTreeMap<String, u64> = ["critical", "high", "medium", "low", "info"].iter().map(|k| (k.to_string(), 0)).collect();
    let mut triage: BTreeMap<String, u64> = BTreeMap::new();
    let day_range = TimeRange { from: r.from / DAY * DAY, to: r.to };
    let days: Vec<i64> = (0..).map(|i| day_range.from + i * DAY).take_while(|t| *t <= r.to).collect();
    let mut by_sev: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for s in &signal_rows {
        *signals.entry(s.severity.clone()).or_default() += 1;
        let key = if s.state == "archived" { s.archive_reason.clone() } else { s.state.clone() };
        *triage.entry(key).or_default() += 1;
        if let Some(i) = days.iter().position(|d| *d == s.day) {
            by_sev.entry(s.severity.clone()).or_insert_with(|| vec![0; days.len()])[i] += 1;
        }
    }
    let triage_order = ["remediated", "true_positive_malicious", "true_positive_benign", "investigated_case_opened", "testing_or_maintenance", "false_positive", "other", "under_review", "open"];
    let triage = triage_order.iter().map(|k| WpCount { name: k.to_string(), count: triage.get(*k).copied().unwrap_or(0) }).collect();
    let (_, failing) = evaluate_findings(&st, org, &access, r, &agents).await?;
    let mut findings: BTreeMap<String, u64> = ["critical", "high", "medium", "low", "info"].iter().map(|k| (k.to_string(), 0)).collect();
    for f in failing.iter().filter(|f| f.status == "open") {
        *findings.entry(f.severity.clone()).or_default() += 1;
    }
    let enabled: HashSet<&str> = rules.iter().filter(|r| r.enabled).map(|r| r.name.as_str()).collect();
    let tactics = TACTICS.iter().map(|(id, name)| WpTactic { id: id.to_string(), name: name.to_string(), rules: AGENT_RULES.iter().filter(|a| a.tactic == *id && enabled.contains(a.name)).count() }).collect();
    let packs = PACKS
        .iter()
        .map(|(id, name, description)| {
            let in_pack: Vec<&str> = AGENT_RULES.iter().filter(|a| a.pack == *id).map(|a| a.name).collect();
            let on = in_pack.iter().filter(|n| enabled.contains(*n)).count();
            WpPack { id: id.to_string(), name: name.to_string(), description: description.to_string(), rules: in_pack.len(), enabled_rules: on, status: if on > 0 { "active".into() } else { "available".into() } }
        })
        .collect();
    let agent_policies = POLICIES
        .iter()
        .map(|(key, name, description)| {
            let id = catalog::policy_id(key);
            WpPolicyCard { rules: rules.iter().filter(|r| r.policy_id == id && r.enabled).count(), id, name: name.to_string(), description: description.to_string() }
        })
        .collect();
    Ok(Json(WpSummary {
        agents: WpAgentCounts { hosts: hosts.len(), hosts_enabled: hosts.iter().filter(|a| a.is_cws_enabled).count(), serverless: serverless.len(), serverless_enabled: serverless.iter().filter(|a| a.is_cws_enabled).count() },
        monitored,
        monitored_series,
        captured,
        captured_series,
        policies: policies.iter().filter(|p| p.enabled).count(),
        agent_rules: rules.iter().filter(|r| r.enabled).count(),
        detection_rules: detection_rules as usize,
        finding_rules: FINDING_RULES.len(),
        signals,
        signal_series: WpSeries { times: days, groups: by_sev.into_iter().map(|(name, values)| WpSeriesGroup { name, values }).collect() },
        triage,
        findings,
        tactics,
        packs,
        agent_policies,
    }))
}

// ================================================================ events

#[derive(Deserialize, ToSchema)]
pub struct WpEventsRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    pub limit: Option<u64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpEvent {
    pub id: String,
    /// Epoch ms.
    pub timestamp: i64,
    pub host: String,
    pub service: String,
    pub title: String,
    pub rule_id: String,
    pub category: String,
    /// The event type (exec, open, dns, …).
    pub event: String,
    pub outcome: String,
    pub process: String,
    pub process_args: String,
    pub user: String,
    pub file: String,
    pub container_id: String,
    pub container_name: String,
    pub image: String,
    pub env: String,
    /// The event as the Agent sent it.
    pub attributes: Value,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpEventsPage {
    pub events: Vec<WpEvent>,
    pub total: u64,
    /// Matching events per bucket and category.
    pub series: WpSeries,
    pub facets: Vec<WpFacet>,
}

#[derive(Serialize, ToSchema)]
pub struct WpFacet {
    /// The search key (`@agent.rule_id`, `host`, …).
    pub key: String,
    pub label: String,
    pub values: Vec<WpCount>,
}

#[derive(clickhouse::Row, Deserialize)]
struct EventRow {
    id: String,
    ts: i64,
    host: String,
    service: String,
    tags_json: String,
    attributes: String,
}

#[derive(clickhouse::Row, Deserialize)]
struct Count {
    n: u64,
}

const FACETS: [(&str, &str); 8] = [
    ("@agent.rule_id", "Agent rule"),
    ("@evt.category", "Category"),
    ("@evt.name", "Event type"),
    ("host", "Host"),
    ("@process.file.name", "Process"),
    ("image_name", "Container image"),
    ("env", "Environment"),
    ("service", "Service"),
];

fn text(v: &Value, path: &str) -> String {
    v.pointer(path).map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| if x.is_null() { String::new() } else { x.to_string() })).unwrap_or_default()
}

pub fn event_of(id: String, ts: i64, host: String, service: String, tags: &Map<String, Value>, attributes: Value) -> WpEvent {
    let tag = |k: &str| tags.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let a = &attributes;
    let args = a.pointer("/process/args").and_then(Value::as_array).map(|x| x.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let file = [text(a, "/file/path"), text(a, "/dns/question/name"), text(a, "/module/name")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
    WpEvent {
        id,
        timestamp: ts,
        host,
        service,
        title: text(a, "/title"),
        rule_id: text(a, "/agent/rule_id"),
        category: text(a, "/evt/category"),
        event: text(a, "/evt/name"),
        outcome: text(a, "/evt/outcome"),
        process: [text(a, "/process/executable/path"), text(a, "/process/comm")].into_iter().find(|s| !s.is_empty()).unwrap_or_default(),
        process_args: args,
        user: text(a, "/process/user"),
        file,
        container_id: text(a, "/container/id"),
        container_name: tag("container_name"),
        image: tag("image_name"),
        env: tag("env"),
        attributes,
    }
}

#[utoipa::path(post, path = "/api/v1/workload-protection/events", operation_id = "workload_protection_events", request_body = WpEventsRequest, responses((status = 200, body = WpEventsPage)), tag = "workload-protection")]
pub async fn events(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Json(req): Json<WpEventsRequest>) -> ApiResult<WpEventsPage> {
    req.range.validate()?;
    let org = ctx.org_id.as_str();
    let limit = req.limit.unwrap_or(100).clamp(1, 1000);
    let s = scope(org, &access, req.range, &source_terms(&req.query), None);
    let mut params = s.params.clone();
    params.push(Param::Int(limit as i64));
    let rows: Vec<EventRow> = bind(
        st.ch(org).query(&format!(
            "SELECT toString(id) AS id, toUnixTimestamp64Milli(timestamp) AS ts, host, service, toJSONString(tags) AS tags_json, attributes FROM {} WHERE {} ORDER BY timestamp DESC LIMIT ?",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all()
    .await?;
    let total: Count = bind(st.ch(org).query(&format!("SELECT count() AS n FROM {} WHERE {}", s.table, s.clause)), &s.params).fetch_one().await?;
    let mut series = event_series(&st, org, &access, &req.query, "@evt.category", req.range).await?;
    for g in &mut series.groups {
        g.name = category_key(&g.name).into();
    }
    let mut facets = Vec::with_capacity(FACETS.len());
    for (key, label) in FACETS {
        let values = counts_by(&st, org, &access, &req.query, key, req.range, 10).await?.into_iter().filter(|c| !c.k.is_empty()).map(|c| WpCount { name: c.k, count: c.n }).collect();
        facets.push(WpFacet { key: key.into(), label: label.into(), values });
    }
    let events = rows
        .into_iter()
        .map(|r| {
            let tags: Map<String, Value> = serde_json::from_str(&r.tags_json).unwrap_or_default();
            let attributes: Value = serde_json::from_str(&r.attributes).unwrap_or(Value::Null);
            event_of(r.id, r.ts, r.host, r.service, &tags, attributes)
        })
        .collect();
    Ok(Json(WpEventsPage { events, total: total.n, series, facets }))
}

// ================================================================ findings

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WpFinding {
    /// The triage key (`workload-finding:<rule>:<resource>`).
    pub key: String,
    pub rule_id: String,
    pub rule_name: String,
    pub severity: String,
    /// host | container
    pub resource_type: String,
    /// The host name, or the container's name (its id when unnamed).
    pub resource: String,
    pub host: String,
    pub container_id: String,
    pub image: String,
    pub service: String,
    pub env: String,
    /// open | muted
    pub status: String,
    /// Matching agent events in the range.
    pub events: u64,
    pub agent_rules: Vec<String>,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpFindingRule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub severity: String,
    pub resource_type: String,
    pub agent_rules: Vec<String>,
    pub failing: usize,
    pub passing: usize,
    pub muted: usize,
}

#[derive(Serialize, ToSchema)]
pub struct WpFindings {
    pub rules: Vec<WpFindingRule>,
    pub findings: Vec<WpFinding>,
}

#[derive(clickhouse::Row, Deserialize)]
struct MatchRow {
    r: String,
    host: String,
    cid: String,
    cname: String,
    image: String,
    env: String,
    svc: String,
    n: u64,
    first: i64,
    last: i64,
}

/// The finding rules evaluated over `r`: (rules with counts, failing findings).
pub async fn evaluate_findings(st: &AppState, org: &str, access: &LogAccess, r: TimeRange, agents: &[CsmAgent]) -> Result<(Vec<WpFindingRule>, Vec<WpFinding>), ApiError> {
    let referenced: Vec<&str> = {
        let mut v: Vec<&str> = FINDING_RULES.iter().flat_map(|f| f.agent_rules.iter().copied()).collect();
        v.sort();
        v.dedup();
        v
    };
    let s = scope(org, access, r, &source_terms(""), None);
    let mut params: Vec<Param> = Vec::new();
    let (rule_e, p) = LogsSchema.field("@agent.rule_id");
    params.extend(p.clone());
    let (cid_e, pc) = LogsSchema.field("@container.id");
    params.extend(pc.clone());
    params.extend(s.params);
    params.extend(p);
    params.extend(referenced.iter().map(|n| Param::Str(n.to_string())));
    let placeholders = vec!["?"; referenced.len()].join(", ");
    let rows: Vec<MatchRow> = bind(
        st.ch(org).query(&format!(
            "SELECT toString({rule_e}) AS r, host, toString({cid_e}) AS cid, any(tags['container_name']) AS cname, any(tags['image_name']) AS image, any(tags['env']) AS env, \
             any(service) AS svc, count() AS n, toInt64(toUnixTimestamp64Milli(min(timestamp))) AS first, toInt64(toUnixTimestamp64Milli(max(timestamp))) AS last \
             FROM {} WHERE {} AND toString({rule_e}) IN ({placeholders}) GROUP BY r, host, cid",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all()
    .await?;
    let triage = super::code_security::triage_map(st, org).await?;
    let mut hosts: HashSet<String> = agents.iter().filter(|a| a.is_cws_enabled).map(|a| a.hostname.clone()).collect();
    let mut containers: HashSet<String> = HashSet::new();
    let containers_seen = counts_by(st, org, access, "", "@container.id", r, 10_000).await?;
    containers.extend(containers_seen.into_iter().filter(|c| !c.k.is_empty()).map(|c| c.k));
    hosts.extend(rows.iter().map(|m| m.host.clone()));
    let mut findings: Vec<WpFinding> = Vec::new();
    let mut out_rules = Vec::with_capacity(FINDING_RULES.len());
    for f in FINDING_RULES {
        // Resource → finding, merging the rule's agent rules.
        let mut by_resource: BTreeMap<String, WpFinding> = BTreeMap::new();
        for m in rows.iter().filter(|m| f.agent_rules.contains(&m.r.as_str())) {
            let container = f.resource_type == "container";
            if container && m.cid.is_empty() {
                continue;
            }
            let id = if container { m.cid.clone() } else { m.host.clone() };
            let key = format!("workload-finding:{}:{id}", f.id);
            let status = triage.get(&key).cloned().unwrap_or_else(|| "open".into());
            let e = by_resource.entry(id.clone()).or_insert_with(|| WpFinding {
                key,
                rule_id: f.id.into(),
                rule_name: f.name.into(),
                severity: f.severity.into(),
                resource_type: f.resource_type.into(),
                resource: if container && !m.cname.is_empty() { m.cname.clone() } else { id.clone() },
                host: m.host.clone(),
                container_id: if container { m.cid.clone() } else { String::new() },
                image: if container { m.image.clone() } else { String::new() },
                service: m.svc.clone(),
                env: m.env.clone(),
                status,
                events: 0,
                agent_rules: vec![],
                first_seen: m.first,
                last_seen: m.last,
            });
            e.events += m.n;
            e.first_seen = e.first_seen.min(m.first);
            e.last_seen = e.last_seen.max(m.last);
            if !e.agent_rules.contains(&m.r) {
                e.agent_rules.push(m.r.clone());
            }
        }
        let resources = if f.resource_type == "container" { containers.len() } else { hosts.len() };
        let failing = by_resource.len();
        out_rules.push(WpFindingRule {
            id: f.id.into(),
            name: f.name.into(),
            description: f.description.into(),
            severity: f.severity.into(),
            resource_type: f.resource_type.into(),
            agent_rules: f.agent_rules.iter().map(|s| s.to_string()).collect(),
            failing,
            passing: resources.saturating_sub(failing),
            muted: by_resource.values().filter(|x| x.status == "muted").count(),
        });
        findings.extend(by_resource.into_values());
    }
    findings.sort_by(|a, b| super::security_rules::severity_rank(&b.severity).cmp(&super::security_rules::severity_rank(&a.severity)).then(b.last_seen.cmp(&a.last_seen)));
    Ok((out_rules, findings))
}

#[utoipa::path(get, path = "/api/v1/workload-protection/findings", operation_id = "workload_protection_findings", params(WpRangeQuery), responses((status = 200, body = WpFindings)), tag = "workload-protection")]
pub async fn findings(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Query(q): Query<WpRangeQuery>) -> ApiResult<WpFindings> {
    let r = q.range()?;
    let agents = cws::agents(&st, &ctx.org_id).await?;
    let (rules, findings) = evaluate_findings(&st, &ctx.org_id, &access, r, &agents).await?;
    Ok(Json(WpFindings { rules, findings }))
}

// ================================================================ agents

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpAgents {
    pub hosts: Vec<CsmAgent>,
    pub serverless: Vec<CsmAgent>,
}

#[utoipa::path(get, path = "/api/v1/workload-protection/agents", operation_id = "workload_protection_agents", responses((status = 200, body = WpAgents)), tag = "workload-protection")]
pub async fn agents(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<WpAgents> {
    let (serverless, hosts): (Vec<CsmAgent>, Vec<CsmAgent>) = cws::agents(&st, &ctx.org_id).await?.into_iter().partition(|a| !a.aws_fargate.is_empty());
    Ok(Json(WpAgents { hosts, serverless }))
}

// ================================================================ policies and agent rules

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    /// Inner lists ANDed, outer list ORed; empty: every host.
    pub host_tags_lists: Vec<Vec<String>>,
    pub is_default: bool,
    pub version: i32,
    pub monitoring: i64,
    pub blocking: i64,
    pub disabled: i64,
    /// Agents with Workload Protection whose tags match the policy.
    pub agents: usize,
    pub updated_at: i64,
    pub updated_by: String,
}

fn matches_tags(lists: &[Vec<String>], tags: &[String]) -> bool {
    lists.is_empty() || lists.iter().any(|l| l.iter().all(|t| tags.contains(t)))
}

fn policy_of(p: &PolicyRow, counts: (i64, i64, i64), agents: &[CsmAgent]) -> WpPolicy {
    let lists: Vec<Vec<String>> = serde_json::from_value(p.host_tags_lists.clone()).unwrap_or_default();
    WpPolicy {
        id: p.id.clone(),
        name: p.name.clone(),
        description: p.description.clone(),
        enabled: p.enabled,
        agents: agents.iter().filter(|a| a.is_cws_enabled && matches_tags(&lists, &a.tags)).count(),
        host_tags_lists: lists,
        is_default: p.is_default,
        version: p.version,
        monitoring: counts.0,
        blocking: counts.1,
        disabled: counts.2,
        updated_at: p.updated_at,
        updated_by: p.updated_by_name.clone(),
    }
}

#[utoipa::path(get, path = "/api/v1/workload-protection/policies", operation_id = "workload_protection_policies", responses((status = 200, body = Vec<WpPolicy>)), tag = "workload-protection")]
pub async fn list_policies(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<WpPolicy>> {
    let rows = cws::policies(&st, &ctx.org_id).await?;
    let counts = cws::rule_counts(&st, &ctx.org_id).await?;
    let agents = cws::agents(&st, &ctx.org_id).await?;
    Ok(Json(rows.iter().map(|p| policy_of(p, counts.get(&p.id).copied().unwrap_or_default(), &agents)).collect()))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpPolicyInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub host_tags_lists: Option<Vec<Vec<String>>>,
}

fn policy_attrs(i: &WpPolicyInput) -> Map<String, Value> {
    let mut a = Map::new();
    if let Some(v) = &i.name {
        a.insert("name".into(), json!(v));
    }
    if let Some(v) = &i.description {
        a.insert("description".into(), json!(v));
    }
    if let Some(v) = i.enabled {
        a.insert("enabled".into(), json!(v));
    }
    if let Some(v) = &i.host_tags_lists {
        a.insert("hostTagsLists".into(), json!(v));
    }
    a
}

async fn policy_response(st: &AppState, org: &str, p: &PolicyRow) -> ApiResult<WpPolicy> {
    let counts = cws::rule_counts(st, org).await?;
    let agents = cws::agents(st, org).await?;
    Ok(Json(policy_of(p, counts.get(&p.id).copied().unwrap_or_default(), &agents)))
}

#[utoipa::path(post, path = "/api/v1/workload-protection/policies", operation_id = "workload_protection_create_policy", request_body = WpPolicyInput, responses((status = 200, body = WpPolicy)), tag = "workload-protection")]
pub async fn create_policy(State(st): State<AppState>, Writer(ctx): Writer, Json(i): Json<WpPolicyInput>) -> ApiResult<WpPolicy> {
    let p = cws::create_policy_row(&st, &ctx, &policy_attrs(&i)).await.map_err(api_err)?;
    policy_response(&st, &ctx.org_id, &p).await
}

#[utoipa::path(patch, path = "/api/v1/workload-protection/policies/{id}", operation_id = "workload_protection_update_policy", params(("id" = String, Path)), request_body = WpPolicyInput, responses((status = 200, body = WpPolicy)), tag = "workload-protection")]
pub async fn update_policy(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(i): Json<WpPolicyInput>) -> ApiResult<WpPolicy> {
    let p = cws::update_policy_row(&st, &ctx, &id, &policy_attrs(&i)).await.map_err(api_err)?;
    policy_response(&st, &ctx.org_id, &p).await
}

#[utoipa::path(delete, path = "/api/v1/workload-protection/policies/{id}", operation_id = "workload_protection_delete_policy", params(("id" = String, Path)), responses((status = 204)), tag = "workload-protection")]
pub async fn delete_policy(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    cws::delete_policy_row(&st, &ctx.org_id, &id).await.map_err(api_err)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpAgentRule {
    pub id: String,
    pub name: String,
    /// A readable name: the catalog's for default rules, the description's first sentence otherwise.
    pub title: String,
    pub description: String,
    pub expression: String,
    /// File Activity | Process Activity | Network Activity | Kernel Activity
    pub category: String,
    pub enabled: bool,
    pub blocking: bool,
    pub silent: bool,
    pub product_tags: Vec<String>,
    pub policy_id: String,
    pub policy_name: String,
    pub is_default: bool,
    pub tactic: String,
    pub technique: String,
    pub pack: String,
    /// The severity of the default detection rule on it ("" for custom rules).
    pub severity: String,
    pub actions: Value,
    /// Events that matched it in the range.
    pub matches: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub updated_by: String,
}

fn rule_of(r: &RuleRow, policies: &HashMap<String, String>, matches: &HashMap<String, u64>) -> WpAgentRule {
    let def = AGENT_RULES.iter().find(|d| d.name == r.name && r.is_default);
    let tag = |prefix: &str| r.product_tags.iter().find_map(|t| t.strip_prefix(prefix).map(str::to_string)).unwrap_or_default();
    WpAgentRule {
        id: r.id.clone(),
        name: r.name.clone(),
        title: def.map(|d| d.title.to_string()).unwrap_or_else(|| r.description.split('.').next().filter(|s| !s.trim().is_empty()).unwrap_or(&r.name).trim().to_string()),
        description: r.description.clone(),
        expression: r.expression.clone(),
        category: catalog::category(&r.expression).into(),
        enabled: r.enabled,
        blocking: r.blocking,
        silent: r.silent.unwrap_or(false),
        product_tags: r.product_tags.clone(),
        policy_id: r.policy_id.clone(),
        policy_name: policies.get(&r.policy_id).cloned().unwrap_or_default(),
        is_default: r.is_default,
        tactic: def.map(|d| d.tactic.to_string()).unwrap_or_else(|| tag("tactic:").split('-').next().unwrap_or("").to_string()),
        technique: def.map(|d| d.technique.to_string()).unwrap_or_else(|| tag("technique:")),
        pack: def.map(|d| d.pack.to_string()).unwrap_or_default(),
        severity: def.map(|d| d.severity.to_string()).unwrap_or_default(),
        actions: r.actions.clone(),
        matches: matches.get(&r.name).copied().unwrap_or(0),
        created_at: r.created_at,
        updated_at: r.updated_at,
        updated_by: r.updated_by_name.clone(),
    }
}

#[derive(Deserialize, IntoParams, Default)]
#[into_params(parameter_in = Query)]
pub struct WpRulesQuery {
    pub policy_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/workload-protection/agent-rules", operation_id = "workload_protection_agent_rules", params(WpRulesQuery), responses((status = 200, body = Vec<WpAgentRule>)), tag = "workload-protection")]
pub async fn list_rules(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Query(q): Query<WpRulesQuery>) -> ApiResult<Vec<WpAgentRule>> {
    let r = WpRangeQuery { from: q.from, to: q.to }.range()?;
    let rows = cws::rules(&st, &ctx.org_id, q.policy_id.as_deref().filter(|p| !p.is_empty())).await?;
    let policies: HashMap<String, String> = cws::policies(&st, &ctx.org_id).await?.into_iter().map(|p| (p.id, p.name)).collect();
    let matches: HashMap<String, u64> = counts_by(&st, &ctx.org_id, &access, "", "@agent.rule_id", r, 1000).await?.into_iter().map(|c| (c.k, c.n)).collect();
    Ok(Json(rows.iter().map(|x| rule_of(x, &policies, &matches)).collect()))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WpAgentRuleInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub expression: Option<String>,
    pub policy_id: Option<String>,
    pub enabled: Option<bool>,
    pub silent: Option<bool>,
    pub product_tags: Option<Vec<String>>,
}

fn rule_attrs(i: &WpAgentRuleInput) -> Map<String, Value> {
    let mut a = Map::new();
    for (k, v) in [("name", &i.name), ("description", &i.description), ("expression", &i.expression), ("policy_id", &i.policy_id)] {
        if let Some(v) = v {
            a.insert(k.into(), json!(v));
        }
    }
    if let Some(v) = i.enabled {
        a.insert("enabled".into(), json!(v));
    }
    if let Some(v) = i.silent {
        a.insert("silent".into(), json!(v));
    }
    if let Some(v) = &i.product_tags {
        a.insert("product_tags".into(), json!(v));
    }
    a
}

async fn rule_response(st: &AppState, org: &str, r: &RuleRow) -> ApiResult<WpAgentRule> {
    let policies: HashMap<String, String> = cws::policies(st, org).await?.into_iter().map(|p| (p.id, p.name)).collect();
    Ok(Json(rule_of(r, &policies, &HashMap::new())))
}

#[utoipa::path(post, path = "/api/v1/workload-protection/agent-rules", operation_id = "workload_protection_create_agent_rule", request_body = WpAgentRuleInput, responses((status = 200, body = WpAgentRule)), tag = "workload-protection")]
pub async fn create_rule(State(st): State<AppState>, Writer(ctx): Writer, Json(i): Json<WpAgentRuleInput>) -> ApiResult<WpAgentRule> {
    let r = cws::create_rule_row(&st, &ctx, &rule_attrs(&i)).await.map_err(api_err)?;
    rule_response(&st, &ctx.org_id, &r).await
}

#[utoipa::path(patch, path = "/api/v1/workload-protection/agent-rules/{id}", operation_id = "workload_protection_update_agent_rule", params(("id" = String, Path)), request_body = WpAgentRuleInput, responses((status = 200, body = WpAgentRule)), tag = "workload-protection")]
pub async fn update_rule(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(i): Json<WpAgentRuleInput>) -> ApiResult<WpAgentRule> {
    let r = cws::update_rule_row(&st, &ctx, &id, &rule_attrs(&i)).await.map_err(api_err)?;
    rule_response(&st, &ctx.org_id, &r).await
}

#[utoipa::path(delete, path = "/api/v1/workload-protection/agent-rules/{id}", operation_id = "workload_protection_delete_agent_rule", params(("id" = String, Path)), responses((status = 204)), tag = "workload-protection")]
pub async fn delete_rule(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    cws::delete_rule_row(&st, &ctx.org_id, &id).await.map_err(api_err)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The enabled policies as Agent policy files (a zip), for Agents without Remote Configuration.
#[utoipa::path(get, path = "/api/v1/workload-protection/policies/download", operation_id = "workload_protection_download_policies", responses((status = 200, content_type = "application/zip", body = Vec<u8>)), tag = "workload-protection")]
pub async fn download(State(st): State<AppState>, ctx: AuthContext) -> Result<Response, ApiError> {
    let bytes = cws::policy_archive(&st, &ctx.org_id).await?;
    Ok(([(header::CONTENT_TYPE, "application/zip"), (header::CONTENT_DISPOSITION, "attachment; filename=\"workload-protection-policies.zip\"")], Bytes::from(bytes)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_types_fall_in_categories() {
        assert_eq!(event_type_category("exec"), "process");
        assert_eq!(event_type_category("open"), "file");
        assert_eq!(event_type_category("dns"), "network");
        assert_eq!(event_type_category("bpf"), "kernel");
        assert_eq!(category_key("File Activity"), "file");
    }

    #[test]
    fn series_buckets_line_up() {
        let r = TimeRange { from: 0, to: 3 * 60_000 };
        let s = WpSeries::from_rows(r, 60_000, vec![BucketCount { b: 60_000, k: "file".into(), n: 2 }, BucketCount { b: 180_000, k: "".into(), n: 1 }]);
        assert_eq!(s.times, vec![0, 60_000, 120_000, 180_000]);
        assert_eq!(s.groups[0].name, "file");
        assert_eq!(s.groups[0].values, vec![0, 2, 0, 0]);
        assert_eq!(s.groups[1].name, "other");
    }

    #[test]
    fn policies_match_host_tags() {
        let tags = vec!["env:prod".to_string(), "team:shop".to_string()];
        assert!(matches_tags(&[], &tags));
        assert!(matches_tags(&[vec!["env:prod".into(), "team:shop".into()]], &tags));
        assert!(!matches_tags(&[vec!["env:prod".into(), "team:ops".into()]], &tags));
        assert!(matches_tags(&[vec!["env:dev".into()], vec!["team:shop".into()]], &tags));
    }

    #[test]
    fn events_read_the_agents_json() {
        let a = json!({"agent": {"rule_id": "shell_in_container"}, "title": "Interactive shell", "evt": {"name": "exec", "category": "Process Activity", "outcome": "Success"},
            "process": {"executable": {"path": "/bin/bash"}, "args": ["-i"], "user": "root"}, "container": {"id": "c0ffee"}});
        let tags: Map<String, Value> = json!({"image_name": "storefront-api", "env": "prod"}).as_object().cloned().unwrap();
        let e = event_of("1".into(), 5, "i-0a1b2c3d4e5f60001".into(), "storefront-api".into(), &tags, a);
        assert_eq!((e.rule_id.as_str(), e.category.as_str(), e.process.as_str(), e.process_args.as_str(), e.container_id.as_str(), e.image.as_str()), ("shell_in_container", "Process Activity", "/bin/bash", "-i", "c0ffee", "storefront-api"));
    }

    /// Agent events in the logs become events, findings and summary counts.
    #[tokio::test]
    async fn agent_events_become_findings() {
        let Some((_, mut states)) = crate::test_support::replicas_with(1, |_| {}).await else { return };
        let st = states.remove(0);
        let org = format!("wp-{}", catalog::random_id());
        sqlx::query("INSERT INTO orgs (id, name) VALUES ($1, $1) ON CONFLICT DO NOTHING").bind(&org).execute(&st.postgres).await.unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let event = json!({"agent": {"rule_id": "compiler_in_container"}, "title": "Compiler run in a container", "evt": {"name": "exec", "category": "Process Activity"},
            "process": {"executable": {"path": "/usr/bin/gcc"}}, "container": {"id": "c0ffee01"}});
        st.ch(&org)
            .query(
                "INSERT INTO logs (org_id, timestamp, status, service, host, source, message, tags, attr, attributes) VALUES \
                 (?, fromUnixTimestamp64Milli(?), 'info', 'runtime-security-agent', 'i-0a1b2c3d4e5f60009', ?, ?, map('container_name', 'storefront-api-7d9', 'image_name', 'storefront-api'), \
                 map('agent.rule_id', 'compiler_in_container', 'evt.category', 'Process Activity', 'evt.name', 'exec', 'container.id', 'c0ffee01'), ?)",
            )
            .bind(&org)
            .bind(now - 60_000)
            .bind(SOURCE)
            .bind(event.to_string())
            .bind(event.to_string())
            .execute()
            .await
            .unwrap();
        let access = LogAccess::resolve(&[], true, &[], &[]);
        let r = TimeRange { from: now - DAY, to: now };
        let (rules, findings) = evaluate_findings(&st, &org, &access, r, &[]).await.unwrap();
        let f = findings.iter().find(|f| f.rule_id == "wp-fr-001").expect("compiler finding");
        assert_eq!((f.resource.as_str(), f.container_id.as_str(), f.image.as_str(), f.status.as_str()), ("storefront-api-7d9", "c0ffee01", "storefront-api", "open"));
        assert_eq!(rules.iter().find(|x| x.id == "wp-fr-001").unwrap().failing, 1);
        let by_cat = counts_by(&st, &org, &access, "", "@evt.category", r, 10).await.unwrap();
        assert_eq!(by_cat.iter().map(|c| (c.k.as_str(), c.n)).collect::<Vec<_>>(), vec![("Process Activity", 1)]);
        let (totals, _) = monitored(&st, &org, r).await.unwrap();
        assert_eq!(totals.values().sum::<u64>(), 0);
        let series = event_series(&st, &org, &access, "", "@evt.category", r).await.unwrap();
        assert_eq!(series.groups.iter().map(|g| g.values.iter().sum::<u64>()).sum::<u64>(), 1);
        // The default detection rule on the compiler agent rule raises a signal for the host.
        super::super::security_rules::ensure_defaults(&st, &org).await.unwrap();
        let rules = super::super::security_rules::load_rules(&st, &org).await.unwrap();
        let rule = rules.iter().find(|x| x.id == catalog::detection_rule_id("compiler_in_container")).expect("default detection rule");
        assert!(rule.active && rule.not_evaluated.is_none());
        let raised = super::super::security_engine::evaluate_rule(&st, &org, &access, rule, &[], now).await.unwrap();
        assert_eq!(raised, 1);
        let (host, rule_type): (Value, String) = sqlx::query_as("SELECT group_values, rule_type FROM security_signals WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!((host["host"].as_str(), rule_type.as_str()), (Some("i-0a1b2c3d4e5f60009"), "workload_security"));
        let rows = cws::rules(&st, &org, None).await.unwrap();
        assert_eq!(rows.len(), AGENT_RULES.len());
        assert_eq!(cws::policies(&st, &org).await.unwrap().len(), POLICIES.len());
    }
}
