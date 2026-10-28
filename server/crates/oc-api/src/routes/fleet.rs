//! Integrations → Fleet Automation: the Datadog Agents and OpenTelemetry Collectors sending to the
//! organization, built only from what they report:
//! - Datadog Agents: their host metadata, check statuses and inventory (Postgres `datadog_hosts`),
//!   with liveness from the metrics they send and the host's service checks (ClickHouse);
//! - OpenTelemetry Collectors: hosts whose host metrics carry `observecraft.collector`
//!   (`otel`, `observecraft-agent`, …), with the version in `observecraft.collector.version` or the
//!   Collector's own `otelcol_*` metrics (`service.version`) when they are sent.
//!
//! Read-only: ObserveCraft does not implement remote configuration or remote upgrades, so the
//! Configuration and Upgrades views show what agents run and how to change it where they're deployed.
//! A fleet is small (one agent per host), so it is assembled in memory after three queries.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{matches, parse_query, Fields, Param, Term};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::logs::{bind, scope, TimeRange};
use super::metrics::{now_ms, points_subquery, with_window};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::{AppState, OrgId};

/// Metrics in the last 10 minutes: the agent is sending.
pub(crate) const METRICS_FRESH_MS: i64 = 10 * 60_000;
/// The Datadog Agent sends its check statuses every 10 minutes and host metadata every 30.
pub(crate) const METADATA_FRESH_MS: i64 = 60 * 60_000;
/// Hosts considered at once.
const MAX_HOSTS: u64 = 10_000;
/// What replaces a secret in a configuration.
pub(crate) const REDACTED: &str = "<redacted>";
/// `observecraft.collector` values of OpenTelemetry Collectors (the Datadog Agent's is `datadog-agent`).
const OTEL_COLLECTORS: &[&str] = &["otel", "observecraft", "observecraft-agent", "observecraft-cluster-agent"];
/// Agent log lines shown in the side panel.
const MAX_LOG_LINES: i64 = 20;

// ================================================================ versions

/// A dotted version (`7.62.2`, `0.163.2`), with its pre-release identifiers (`rc.3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    core: Vec<u64>,
    pre: Vec<String>,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let len = self.core.len().max(other.core.len());
        let part = |v: &Version, i: usize| v.core.get(i).copied().unwrap_or(0);
        for i in 0..len {
            let o = part(self, i).cmp(&part(other, i));
            if o.is_ne() {
                return o;
            }
        }
        // A release is newer than its pre-releases; pre-release identifiers compare as in semver.
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if o.is_ne() {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The version part of what an agent reports: `otelcol-contrib 0.163.2` → `0.163.2`.
pub(crate) fn version_label(s: &str) -> Option<String> {
    s.split_whitespace().last().map(|v| v.trim_start_matches('v').to_string()).filter(|v| !v.is_empty())
}

/// `7.62.2`, `v0.163.2`, `7.63.0-rc.3`, `7.58.2-1` (a package revision, not a pre-release), `7.60.0+git.12`.
pub(crate) fn parse_version(s: &str) -> Option<Version> {
    let label = version_label(s)?;
    let token = label.split('+').next()?;
    let (core, pre) = match token.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (token, None),
    };
    let core: Vec<u64> = core.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    if core.len() < 2 {
        return None;
    }
    let pre = pre.filter(|p| !p.chars().all(|c| c.is_ascii_digit())).map(|p| p.split('.').map(str::to_string).collect()).unwrap_or_default();
    Some(Version { core, pre })
}

/// The newest release among `versions` (pre-releases only when nothing else is known), as labelled.
pub(crate) fn latest_version<'a>(versions: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let parsed: Vec<(Version, String)> = versions.into_iter().filter_map(|v| Some((parse_version(v)?, version_label(v)?))).collect();
    let releases = parsed.iter().filter(|(v, _)| v.pre.is_empty());
    let best = releases.max_by(|a, b| a.0.cmp(&b.0)).or_else(|| parsed.iter().max_by(|a, b| a.0.cmp(&b.0)));
    best.map(|(_, label)| label.clone())
}

/// Whether `version` is older than `latest`; unknown versions never are.
pub(crate) fn is_outdated(version: &str, latest: &str) -> bool {
    match (parse_version(version), parse_version(latest)) {
        (Some(v), Some(l)) => v < l,
        _ => false,
    }
}

// ================================================================ configuration sanitising

/// Key fragments of settings that hold secrets.
const SECRET_KEYS: &[&str] = &["api_key", "apikey", "app_key", "appkey", "application_key", "password", "passwd", "token", "secret", "private_key", "credential", "authorization"];

pub(crate) fn is_secret_key(key: &str) -> bool {
    let k = key.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase().replace('-', "_");
    SECRET_KEYS.iter().any(|s| k.contains(s))
}

/// `https://user:pass@proxy:3128` → `https://<redacted>@proxy:3128`, everywhere in `s`.
pub(crate) fn redact_url_credentials(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail.find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'').unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                out.push_str(REDACTED);
                rest = &tail[at..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

/// A copy of `v` with secrets replaced: values of secret-looking keys (booleans and empty values
/// stay, they hold no secret) and credentials in URLs.
pub(crate) fn sanitize(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let blank = matches!(v, Value::Null | Value::Bool(_)) || v.as_str().is_some_and(|s| s.trim().is_empty());
                    (k.clone(), if is_secret_key(k) && !blank { Value::String(REDACTED.into()) } else { sanitize(v) })
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(sanitize).collect()),
        Value::String(s) => Value::String(redact_url_credentials(s)),
        v => v.clone(),
    }
}

/// Configuration text as the Agent reports it (YAML): parsed and sanitised, or redacted line by line
/// (`key: value` and `KEY=value`) when it isn't a YAML mapping.
pub(crate) fn sanitize_config_text(text: &str) -> Value {
    if let Ok(v) = serde_yaml_ng::from_str::<Value>(text) {
        if v.is_object() {
            return sanitize(&v);
        }
    }
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let body = line.trim_start();
            let indent = &line[..line.len() - body.len()];
            let (dash, item) = body.strip_prefix("- ").map_or(("", body), |b| ("- ", b));
            for sep in [':', '='] {
                if let Some((k, v)) = item.split_once(sep) {
                    if is_secret_key(k) && !v.trim().is_empty() {
                        let space = if sep == ':' { " " } else { "" };
                        return format!("{indent}{dash}{k}{sep}{space}{REDACTED}");
                    }
                }
            }
            redact_url_credentials(line)
        })
        .collect();
    Value::String(lines.join("\n"))
}

// ================================================================ checks and status

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FleetAgentKind {
    DatadogAgent,
    OtelCollector,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FleetStatus {
    /// Sending metrics, no failing check.
    Ok,
    /// Reporting, but its metrics stopped or a check fails.
    Issues,
    /// Neither metrics nor metadata recently.
    Offline,
}

/// Ordered by severity.
#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FleetCheckStatus {
    Ok,
    Warning,
    Error,
}

/// A check the Agent runs, over its instances.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FleetCheck {
    pub name: String,
    /// The worst status of its instances.
    pub status: FleetCheckStatus,
    /// The message of the worst instance, when it has one.
    pub message: Option<String>,
    pub instances: u32,
    /// The check's configuration failed to load.
    pub load_error: bool,
}

/// The last status of a service check on the agent's host.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct FleetServiceCheck {
    pub name: String,
    /// OK, WARNING, CRITICAL or UNKNOWN.
    pub status: String,
}

/// One check instance as the intake stores it (`datadog_hosts.agent_check_runs`).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub(crate) struct StoredRun {
    name: String,
    instance: String,
    status: String,
    message: String,
}

fn check_status(s: &str) -> FleetCheckStatus {
    match s.to_ascii_uppercase().as_str() {
        "ERROR" | "CRITICAL" => FleetCheckStatus::Error,
        "WARNING" | "WARN" => FleetCheckStatus::Warning,
        _ => FleetCheckStatus::Ok,
    }
}

/// One entry per check: its instances, worst status and that instance's message.
pub(crate) fn summarize_checks(runs: &[StoredRun]) -> Vec<FleetCheck> {
    let mut by_name: BTreeMap<&str, FleetCheck> = BTreeMap::new();
    for r in runs.iter().filter(|r| !r.name.is_empty()) {
        let status = check_status(&r.status);
        let message = Some(r.message.trim().to_string()).filter(|m| !m.is_empty());
        let c = by_name.entry(&r.name).or_insert(FleetCheck { name: r.name.clone(), status, message: message.clone(), instances: 0, load_error: false });
        c.instances += 1;
        c.load_error |= r.instance == "initialization";
        if status > c.status || (status == c.status && c.message.is_none()) {
            c.status = status;
            c.message = message;
        }
    }
    by_name.into_values().collect()
}

fn service_check_status(v: u8) -> &'static str {
    match v {
        0 => "OK",
        1 => "WARNING",
        2 => "CRITICAL",
        _ => "UNKNOWN",
    }
}

/// An agent's status and what is wrong, at `now`: metrics in the last 10 minutes mean it is sending;
/// metadata (Datadog Agents only) in the last hour means it is running. Failing checks and
/// service checks in WARNING or CRITICAL are issues too.
pub(crate) fn agent_status(now: i64, metadata_at: Option<i64>, metrics_at: Option<i64>, checks: &[FleetCheck], service_checks: &[FleetServiceCheck]) -> (FleetStatus, Vec<String>) {
    let fresh = |t: Option<i64>, window: i64| t.is_some_and(|t| t >= now - window);
    let metrics_ok = fresh(metrics_at, METRICS_FRESH_MS);
    if !metrics_ok && !fresh(metadata_at, METADATA_FRESH_MS) {
        return (FleetStatus::Offline, vec!["Not reporting: no metrics in the last 10 minutes".into()]);
    }
    let mut issues = Vec::new();
    if !metrics_ok {
        issues.push(match metrics_at {
            Some(_) => "Metadata still arrives, but no metrics in the last 10 minutes".to_string(),
            None => "Metadata arrives, but no metrics from this agent".to_string(),
        });
    }
    for c in checks.iter().filter(|c| c.status != FleetCheckStatus::Ok) {
        let what = if c.load_error { "failed to load".to_string() } else { format!("is in {}", if c.status == FleetCheckStatus::Error { "ERROR" } else { "WARNING" }) };
        issues.push(match &c.message {
            Some(m) => format!("Check {} {what}: {}", c.name, m.chars().take(200).collect::<String>()),
            None => format!("Check {} {what}", c.name),
        });
    }
    for s in service_checks.iter().filter(|s| s.status == "WARNING" || s.status == "CRITICAL") {
        issues.push(format!("Service check {} is {}", s.name, s.status));
    }
    (if issues.is_empty() { FleetStatus::Ok } else { FleetStatus::Issues }, issues)
}

// ================================================================ coverage

#[derive(Serialize, ToSchema, Debug, PartialEq, Default, Clone)]
pub struct FleetCoverage {
    /// Hosts seen in host or cloud metrics, plus hosts with an agent.
    pub hosts: usize,
    /// Of those, hosts running an agent.
    pub covered: usize,
    /// null without hosts.
    pub pct: Option<f64>,
}

pub(crate) fn coverage(metric_hosts: &BTreeSet<String>, agent_hosts: &BTreeSet<String>) -> FleetCoverage {
    let hosts = metric_hosts.union(agent_hosts).count();
    let covered = agent_hosts.len();
    FleetCoverage { hosts, covered, pct: (hosts > 0).then(|| (covered as f64 * 1000.0 / hosts as f64).round() / 10.0) }
}

// ================================================================ configuration documents and profiles

/// Host metadata fields that describe the Agent's setup (not the host).
const AGENT_SETUP_KEYS: &[&str] = &["agent-flavor", "agentFlavor", "install-method", "logs", "otlp", "proxy-info", "python"];
/// Inventory settings that make up a profile (features, endpoints, install method).
const INVENTORY_PREFIXES: &[&str] = &["feature_", "config_", "install_method_", "flavor"];

/// What an agent reports about its configuration, sanitised: its setup (host metadata), its
/// integrations, its inventory settings and, when the Agent sends it, its configuration.
/// None when the agent reports nothing about it (OpenTelemetry Collectors).
pub(crate) fn config_document(payload: Option<&Value>, inventory: Option<&Value>, integrations: &[String]) -> Option<Value> {
    let payload = payload?;
    let mut doc = serde_json::Map::new();
    let setup: serde_json::Map<String, Value> = AGENT_SETUP_KEYS.iter().filter_map(|k| payload.get(*k).map(|v| (k.replace("agentFlavor", "agent-flavor"), sanitize(v)))).collect();
    doc.insert("agent".into(), Value::Object(setup));
    doc.insert("integrations".into(), Value::Array(integrations.iter().cloned().map(Value::String).collect()));
    if let Some(inv) = inventory.and_then(Value::as_object) {
        let settings: serde_json::Map<String, Value> =
            inv.iter().filter(|(k, v)| INVENTORY_PREFIXES.iter().any(|p| k.starts_with(p)) && !v.is_object()).map(|(k, v)| (k.clone(), sanitize(v))).collect();
        doc.insert("inventory".into(), Value::Object(settings));
        if let Some(text) = inv.get("full_configuration").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
            doc.insert("configuration".into(), sanitize_config_text(text));
        }
    }
    Some(Value::Object(doc))
}

fn flatten(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                flatten(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, v, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        Value::Null => {}
        v => {
            out.insert(prefix.to_string(), v.to_string());
        }
    }
}

/// The settings agents with the same profile share: the setup and inventory parts of the
/// configuration document (the full configuration names the host, so it isn't part of it), without
/// the versions of the Agent's parts, which change with every upgrade.
pub(crate) fn profile_settings(doc: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(doc) = doc {
        for section in ["agent", "inventory"] {
            if let Some(v) = doc.get(section) {
                flatten("", v, &mut out);
            }
        }
    }
    out.retain(|k, _| k != "python" && !k.ends_with("version"));
    out
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct FleetProfileAgent {
    pub id: String,
    pub hostname: String,
    pub version: Option<String>,
}

/// Agents of one kind with the same integrations and settings.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct FleetProfile {
    /// Stable for the same kind, integrations and settings.
    pub id: String,
    pub kind: FleetAgentKind,
    pub integrations: Vec<String>,
    pub settings: BTreeMap<String, String>,
    pub agents: Vec<FleetProfileAgent>,
    /// The versions its agents run, newest first.
    pub versions: Vec<String>,
}

/// What agents of one profile share: kind, integrations and settings.
type ProfileKey = (FleetAgentKind, Vec<String>, Vec<(String, String)>);

/// Groups agents by (kind, integrations, settings), the largest profiles first.
pub(crate) fn group_profiles(agents: &[(&FleetAgent, &BTreeMap<String, String>)]) -> Vec<FleetProfile> {
    let mut groups: BTreeMap<ProfileKey, Vec<&FleetAgent>> = BTreeMap::new();
    for (a, settings) in agents {
        let key = (a.kind, a.integrations.clone(), settings.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        groups.entry(key).or_default().push(a);
    }
    let mut out: Vec<FleetProfile> = groups
        .into_iter()
        .map(|((kind, integrations, settings), members)| {
            let mut hash = Sha256::new();
            hash.update(serde_json::to_vec(&(kind, &integrations, &settings)).unwrap_or_default());
            let id = hash.finalize().iter().take(4).map(|b| format!("{b:02x}")).collect();
            let mut versions: Vec<String> = members.iter().filter_map(|a| a.version.clone()).collect::<BTreeSet<_>>().into_iter().collect();
            versions.sort_by_key(|v| std::cmp::Reverse(parse_version(v)));
            FleetProfile {
                id,
                kind,
                integrations,
                settings: settings.into_iter().collect(),
                agents: members.iter().map(|a| FleetProfileAgent { id: a.id.clone(), hostname: a.hostname.clone(), version: a.version.clone() }).collect(),
                versions,
            }
        })
        .collect();
    out.sort_by(|a, b| b.agents.len().cmp(&a.agents.len()).then(a.kind.cmp(&b.kind)).then(a.id.cmp(&b.id)));
    out
}

// ================================================================ agents

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FleetAgent {
    /// `datadog_agent:<hostname>` or `otel_collector:<hostname>`.
    pub id: String,
    pub hostname: String,
    pub kind: FleetAgentKind,
    pub version: Option<String>,
    /// Older than the newest version of its kind in the fleet.
    pub outdated: bool,
    /// linux, windows or darwin.
    pub os: Option<String>,
    /// Distribution and version, e.g. "ubuntu 24.04".
    pub platform: Option<String>,
    pub status: FleetStatus,
    pub issues: Vec<String>,
    /// The integrations its checks belong to (`system` for the host checks).
    pub integrations: Vec<String>,
    pub env: Option<String>,
    pub cluster: Option<String>,
    /// Epoch ms of its latest metadata or metric.
    pub last_reported: Option<i64>,
    /// How it was installed, when it says: helm, datadog-operator, install_script_agent7, observecraft-agent, …
    pub install_method: Option<String>,
}

/// Everything known about one agent.
struct AgentRow {
    agent: FleetAgent,
    checks: Vec<FleetCheck>,
    service_checks: Vec<FleetServiceCheck>,
    config: Option<Value>,
}

fn non_empty(s: &str) -> Option<String> {
    Some(s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `env:prod` → prod, from host tags.
fn tag_value(tags: &[String], keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| tags.iter().find_map(|t| t.split_once(':').filter(|(tk, v)| tk == k && !v.is_empty()).map(|(_, v)| v.to_string())))
}

/// Facet values of an agent (integrations are matched apart: an agent has several).
fn agent_tags(a: &FleetAgent) -> HashMap<String, String> {
    let mut tags = HashMap::from([
        ("type".to_string(), super::hosts::enum_str(&a.kind)),
        ("status".to_string(), super::hosts::enum_str(&a.status)),
        ("host".to_string(), a.hostname.clone()),
        ("outdated".to_string(), a.outdated.to_string()),
    ]);
    for (k, v) in [("version", &a.version), ("os", &a.os), ("platform", &a.platform), ("cluster", &a.cluster), ("env", &a.env), ("install_method", &a.install_method)] {
        if let Some(v) = v {
            tags.insert(k.to_string(), v.clone());
        }
    }
    tags
}

/// `type:otel_collector version:7.62.2 os:linux status:issues integration:redisdb cluster:x env:prod` + free text.
fn agent_matches(a: &FleetAgent, terms: &[Term]) -> bool {
    let (multi, rest): (Vec<&Term>, Vec<&Term>) = terms.iter().partition(|t| matches!(t, Term::Tag { key, .. } if key == "integration"));
    for t in multi {
        if let Term::Tag { values, negate, .. } = t {
            if values.iter().any(|v| a.integrations.contains(v)) == *negate {
                return false;
            }
        }
    }
    let rest: Vec<Term> = rest.into_iter().cloned().collect();
    let tags = agent_tags(a);
    let empty = HashMap::new();
    let text = format!("{} {} {}", a.hostname, a.version.as_deref().unwrap_or(""), a.platform.as_deref().unwrap_or(""));
    // `status` is one of the matcher's own fields.
    let status = super::hosts::enum_str(&a.status);
    matches(&rest, &Fields { service: "", status: &status, host: &a.hostname, source: "", message: &text, tags: &tags, attr: &empty, index: None })
}

const FACET_KEYS: &[&str] = &["type", "status", "version", "os", "integration", "cluster", "env", "install_method"];

#[derive(Serialize, ToSchema)]
pub struct FleetFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
pub struct FleetFacet {
    pub key: String,
    pub values: Vec<FleetFacetValue>,
}

fn facets(agents: &[&FleetAgent]) -> Vec<FleetFacet> {
    FACET_KEYS
        .iter()
        .map(|k| {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for a in agents {
                if *k == "integration" {
                    for i in &a.integrations {
                        *counts.entry(i.clone()).or_default() += 1;
                    }
                } else if let Some(v) = agent_tags(a).remove(*k) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let mut values: Vec<FleetFacetValue> = counts.into_iter().map(|(value, count)| FleetFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            FleetFacet { key: k.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect()
}

#[derive(clickhouse::Row, Deserialize)]
struct SeriesHost {
    host: String,
    collector: String,
    seen: i64,
    version: String,
    cluster: String,
    env: String,
    os: String,
    platform: String,
    host_metrics: u8,
    cloud_metrics: u8,
}

#[derive(clickhouse::Row, Deserialize)]
struct ServiceCheckRow {
    host: String,
    check: String,
    status: f64,
}

#[derive(sqlx::FromRow)]
struct DdHost {
    host: String,
    agent_version: String,
    os: String,
    platform: String,
    host_tags: Vec<String>,
    agent_checks: Vec<String>,
    agent_check_runs: sqlx::types::Json<Vec<StoredRun>>,
    payload: sqlx::types::Json<Value>,
    agent_inventory: Option<sqlx::types::Json<Value>>,
    metadata_at: i64,
}

/// Every agent seen around `range`, and the hosts seen in host or cloud metrics (for coverage).
async fn load_fleet(st: &AppState, org: &str, range: TimeRange) -> Result<(Vec<AgentRow>, BTreeSet<String>), ApiError> {
    range.validate()?;
    let now = range.to.min(now_ms());
    let series = st
        .ch(org)
        .query(
            // One row per (host, sender); the Collector's own otelcol_* metrics name its version.
            "SELECT ResourceAttributes['host.name'] AS host, \
             if(startsWith(MetricName, 'otelcol'), 'otelcol', ResourceAttributes['observecraft.collector']) AS collector, \
             toInt64(toUnixTimestamp(max(LastSeen))) * 1000 AS seen, \
             argMaxIf(if(startsWith(MetricName, 'otelcol'), ResourceAttributes['service.version'], ResourceAttributes['observecraft.collector.version']), LastSeen, \
                      if(startsWith(MetricName, 'otelcol'), ResourceAttributes['service.version'], ResourceAttributes['observecraft.collector.version']) != '') AS version, \
             argMaxIf(ResourceAttributes['k8s.cluster.name'], LastSeen, ResourceAttributes['k8s.cluster.name'] != '') AS cluster, \
             argMaxIf(if(ResourceAttributes['deployment.environment.name'] != '', ResourceAttributes['deployment.environment.name'], ResourceAttributes['deployment.environment']), LastSeen, \
                      ResourceAttributes['deployment.environment.name'] != '' OR ResourceAttributes['deployment.environment'] != '') AS env, \
             argMaxIf(ResourceAttributes['os.type'], LastSeen, ResourceAttributes['os.type'] != '') AS os, \
             argMaxIf(ResourceAttributes['os.description'], LastSeen, ResourceAttributes['os.description'] != '') AS platform, \
             toUInt8(countIf(startsWith(MetricName, 'system.')) > 0) AS host_metrics, \
             toUInt8(countIf(startsWith(MetricName, 'aws.ec2.') OR startsWith(MetricName, 'gcp.gce.') OR startsWith(MetricName, 'azure.vm.')) > 0) AS cloud_metrics \
             FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND LastSeen >= toDateTime(intDiv(?, 1000)) AND LastSeen <= toDateTime(intDiv(?, 1000) + 60) AND host != '' \
             GROUP BY host, collector LIMIT ?",
        )
        .bind(range.from)
        .bind(range.to)
        .bind(MAX_HOSTS)
        .fetch_all::<SeriesHost>();
    // Service checks' last status in the last 15 minutes of the range.
    let checks_from = range.from.max(now - 15 * 60_000);
    let src = points_subquery("gauge", &["service_check.status"]);
    let service_checks = with_window(st.ch(org), checks_from, range.to)
        .query(&format!(
            "SELECT ResourceAttributes['host.name'] AS host, Attributes['check'] AS check, argMax(Value, TimeUnix) AS status \
             FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'service_check.status' AND host != '' AND check != '' GROUP BY host, check"
        ))
        .fetch_all::<ServiceCheckRow>();
    let dd_hosts = sqlx::query_as::<_, DdHost>(
        "SELECT host, agent_version, os, platform, host_tags, agent_checks, agent_check_runs, payload, agent_inventory, \
         (EXTRACT(EPOCH FROM GREATEST(updated_at, COALESCE(checks_at, updated_at))) * 1000)::BIGINT AS metadata_at \
         FROM datadog_hosts WHERE org_id = $1",
    )
    .bind(org)
    .fetch_all(&st.postgres);
    let (series, service_checks, dd_hosts) =
        tokio::try_join!(async { series.await.map_err(ApiError::from) }, async { service_checks.await.map_err(ApiError::from) }, async { dd_hosts.await.map_err(ApiError::from) })?;

    let mut svc: HashMap<String, Vec<FleetServiceCheck>> = HashMap::new();
    for r in service_checks {
        svc.entry(r.host).or_default().push(FleetServiceCheck { name: r.check, status: service_check_status(r.status.clamp(0.0, 3.0) as u8).into() });
    }
    for v in svc.values_mut() {
        v.sort_by(|a, b| a.name.cmp(&b.name));
    }
    let metric_hosts: BTreeSet<String> = series.iter().filter(|s| s.collector != "otelcol" && (s.host_metrics == 1 || s.cloud_metrics == 1)).map(|s| s.host.clone()).collect();
    let dd_seen: HashMap<&str, i64> = series.iter().filter(|s| s.collector == "datadog-agent").map(|s| (s.host.as_str(), s.seen)).collect();
    let otelcol: HashMap<&str, &SeriesHost> = series.iter().filter(|s| s.collector == "otelcol").map(|s| (s.host.as_str(), s)).collect();

    let mut rows = Vec::new();
    for d in dd_hosts {
        let metrics_at = dd_seen.get(d.host.as_str()).copied();
        if d.metadata_at < range.from - METADATA_FRESH_MS && metrics_at.is_none() {
            continue;
        }
        let checks = summarize_checks(&d.agent_check_runs.0);
        let service_checks = svc.get(&d.host).cloned().unwrap_or_default();
        let (status, issues) = agent_status(now, Some(d.metadata_at), metrics_at, &checks, &service_checks);
        let payload = &d.payload.0;
        let inventory = d.agent_inventory.as_ref().map(|i| &i.0);
        let integrations = super::hosts::host_apps(&d.agent_checks);
        let install_method = payload
            .pointer("/install-method/tool")
            .and_then(Value::as_str)
            .and_then(non_empty)
            .or_else(|| inventory.and_then(|i| i.get("install_method_tool")).and_then(Value::as_str).and_then(non_empty));
        let config = config_document(Some(payload), inventory, &integrations);
        rows.push(AgentRow {
            agent: FleetAgent {
                id: format!("datadog_agent:{}", d.host),
                kind: FleetAgentKind::DatadogAgent,
                version: version_label(&d.agent_version),
                outdated: false,
                os: non_empty(&d.os),
                platform: non_empty(&d.platform),
                status,
                issues,
                integrations,
                env: tag_value(&d.host_tags, &["env"]),
                cluster: tag_value(&d.host_tags, &["kube_cluster_name", "cluster_name", "eks_cluster-name"]),
                last_reported: Some(metrics_at.map_or(d.metadata_at, |m| m.max(d.metadata_at))),
                install_method,
                hostname: d.host,
            },
            checks,
            service_checks,
            config,
        });
    }
    for s in series.iter().filter(|s| OTEL_COLLECTORS.contains(&s.collector.as_str()) && s.host_metrics == 1) {
        let own = otelcol.get(s.host.as_str());
        let seen = own.map_or(s.seen, |o| o.seen.max(s.seen));
        let version = non_empty(&s.version).or_else(|| own.and_then(|o| non_empty(&o.version))).and_then(|v| version_label(&v));
        let service_checks = svc.get(&s.host).cloned().unwrap_or_default();
        let (status, issues) = agent_status(now, None, Some(seen), &[], &service_checks);
        rows.push(AgentRow {
            agent: FleetAgent {
                id: format!("otel_collector:{}", s.host),
                hostname: s.host.clone(),
                kind: FleetAgentKind::OtelCollector,
                version,
                outdated: false,
                os: non_empty(&s.os),
                platform: non_empty(&s.platform),
                status,
                issues,
                integrations: vec![],
                env: non_empty(&s.env),
                cluster: non_empty(&s.cluster),
                last_reported: Some(seen),
                install_method: (s.collector == "observecraft-agent").then(|| "observecraft-agent".to_string()),
            },
            checks: vec![],
            service_checks,
            config: None,
        });
    }
    // Outdated: older than the newest version of the same kind in the fleet.
    let latest = latest_by_kind(rows.iter().map(|r| &r.agent));
    for r in &mut rows {
        if let (Some(v), Some(l)) = (&r.agent.version, latest.get(&r.agent.kind)) {
            r.agent.outdated = is_outdated(v, l);
        }
    }
    rows.sort_by(|a, b| a.agent.hostname.cmp(&b.agent.hostname).then(a.agent.kind.cmp(&b.agent.kind)));
    Ok((rows, metric_hosts))
}

fn latest_by_kind<'a>(agents: impl Iterator<Item = &'a FleetAgent> + Clone) -> HashMap<FleetAgentKind, String> {
    [FleetAgentKind::DatadogAgent, FleetAgentKind::OtelCollector]
        .into_iter()
        .filter_map(|k| latest_version(agents.clone().filter(|a| a.kind == k).filter_map(|a| a.version.as_deref())).map(|v| (k, v)))
        .collect()
}

// ================================================================ fleet view

#[derive(Deserialize, ToSchema)]
pub struct FleetSearchRequest {
    pub range: TimeRange,
    /// `type:datadog_agent status:issues version:7.62.2 os:linux integration:redisdb cluster:x env:prod outdated:true` + free text on the hostname.
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct FleetSummary {
    pub agents: usize,
    pub outdated: usize,
    /// Reporting, with issues.
    pub with_issues: usize,
    pub offline: usize,
    pub coverage: FleetCoverage,
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct FleetLatest {
    pub datadog_agent: Option<String>,
    pub otel_collector: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct FleetVersion {
    pub kind: FleetAgentKind,
    /// null for agents that don't report their version.
    pub version: Option<String>,
    pub count: usize,
    pub outdated: bool,
}

#[derive(Serialize, ToSchema)]
pub struct FleetPage {
    /// False when no agent reported at all (show the empty state).
    pub enabled: bool,
    /// Over the whole fleet, whatever the query.
    pub summary: FleetSummary,
    pub latest: FleetLatest,
    /// Agents per kind and version, newest first; over the whole fleet.
    pub versions: Vec<FleetVersion>,
    pub facets: Vec<FleetFacet>,
    /// The agents matching the query.
    pub agents: Vec<FleetAgent>,
}

fn versions(agents: &[&FleetAgent]) -> Vec<FleetVersion> {
    let mut counts: BTreeMap<(FleetAgentKind, Option<String>), (usize, bool)> = BTreeMap::new();
    for a in agents {
        let e = counts.entry((a.kind, a.version.clone())).or_default();
        e.0 += 1;
        e.1 |= a.outdated;
    }
    let mut out: Vec<FleetVersion> = counts.into_iter().map(|((kind, version), (count, outdated))| FleetVersion { kind, version, count, outdated }).collect();
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| b.version.as_deref().and_then(parse_version).cmp(&a.version.as_deref().and_then(parse_version))));
    out
}

/// The fleet's agents and collectors, with summary cards, versions and facets.
#[utoipa::path(post, path = "/api/v1/fleet/agents", operation_id = "fleet_agents", request_body = FleetSearchRequest, responses((status = 200, body = FleetPage)), tag = "fleet")]
pub async fn agents(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<FleetSearchRequest>) -> ApiResult<FleetPage> {
    let (rows, metric_hosts) = load_fleet(&st, &org, req.range).await?;
    let all: Vec<&FleetAgent> = rows.iter().map(|r| &r.agent).collect();
    let agent_hosts: BTreeSet<String> = all.iter().map(|a| a.hostname.clone()).collect();
    let latest = latest_by_kind(all.iter().copied());
    let terms = parse_query(&req.query);
    Ok(Json(FleetPage {
        enabled: !all.is_empty(),
        summary: FleetSummary {
            agents: all.len(),
            outdated: all.iter().filter(|a| a.outdated).count(),
            with_issues: all.iter().filter(|a| a.status == FleetStatus::Issues).count(),
            offline: all.iter().filter(|a| a.status == FleetStatus::Offline).count(),
            coverage: coverage(&metric_hosts, &agent_hosts),
        },
        latest: FleetLatest { datadog_agent: latest.get(&FleetAgentKind::DatadogAgent).cloned(), otel_collector: latest.get(&FleetAgentKind::OtelCollector).cloned() },
        versions: versions(&all),
        facets: facets(&all),
        agents: all.iter().filter(|a| agent_matches(a, &terms)).map(|a| (*a).clone()).collect(),
    }))
}

// ================================================================ agent panel

#[derive(Deserialize, ToSchema)]
pub struct FleetAgentRequest {
    pub range: TimeRange,
    /// The agent's `id` from the fleet list.
    pub id: String,
}

#[derive(Serialize, ToSchema)]
pub struct FleetLogLine {
    /// Epoch ms.
    pub timestamp: i64,
    pub status: String,
    pub message: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FleetAgentDetail {
    pub agent: FleetAgent,
    /// The checks the Datadog Agent runs, with their status.
    pub checks: Vec<FleetCheck>,
    /// Service checks on the agent's host (`datadog.agent.up`, integrations' `*.can_connect`, …).
    pub service_checks: Vec<FleetServiceCheck>,
    /// What the agent reports about its configuration, as YAML with secrets redacted; null when it
    /// reports nothing (OpenTelemetry Collectors).
    pub config_yaml: Option<String>,
    /// Whether the Agent sent its full configuration (`inventories_configuration_enabled`).
    pub full_config_reported: bool,
    /// Recent errors in the agent's own logs, newest first (only logs the viewer may read).
    pub errors: Vec<FleetLogLine>,
    /// The Log Explorer query for the agent's own logs.
    pub logs_query: String,
}

/// The `service:` values of an agent's own logs.
fn agent_log_services(kind: FleetAgentKind) -> Vec<String> {
    match kind {
        FleetAgentKind::DatadogAgent => vec!["datadog-agent".into()],
        FleetAgentKind::OtelCollector => vec!["otelcol-contrib".into(), "otelcol".into(), "observecraft-agent".into()],
    }
}

/// One agent: its checks, sanitised configuration and recent errors from its own logs.
#[utoipa::path(post, path = "/api/v1/fleet/agent", operation_id = "fleet_agent", request_body = FleetAgentRequest, responses((status = 200, body = FleetAgentDetail), (status = 404, body = crate::error::ErrorBody)), tag = "fleet")]
pub async fn agent(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<FleetAgentRequest>) -> ApiResult<FleetAgentDetail> {
    let (rows, _) = load_fleet(&st, &org, req.range).await?;
    let row = rows.into_iter().find(|r| r.agent.id == req.id).ok_or_else(|| ApiError::NotFound(format!("Agent {} has not reported in this time range", req.id)))?;
    let services = agent_log_services(row.agent.kind);
    let terms = vec![
        Term::Tag { key: "service".into(), values: services.clone(), negate: false },
        Term::Tag { key: "host".into(), values: vec![row.agent.hostname.clone()], negate: false },
        Term::Tag { key: "status".into(), values: vec!["error".into(), "critical".into(), "alert".into(), "emergency".into()], negate: false },
    ];
    let s = scope(&org, &access, req.range, &terms, None);
    let mut params = s.params;
    params.push(Param::Int(MAX_LOG_LINES));
    #[derive(clickhouse::Row, Deserialize)]
    struct LogRow {
        ts: i64,
        status: String,
        message: String,
    }
    let errors = bind(
        st.ch(&org).query(&format!(
            "SELECT toInt64(toUnixTimestamp64Milli(timestamp)) AS ts, toString(status) AS status, substringUTF8(message, 1, 500) AS message FROM {} WHERE {} ORDER BY timestamp DESC LIMIT ?",
            s.table, s.clause
        )),
        &params,
    )
    .fetch_all::<LogRow>()
    .await?;
    let full_config_reported = row.config.as_ref().is_some_and(|c| c.get("configuration").is_some());
    let config_yaml = row.config.as_ref().map(serde_yaml_ng::to_string).transpose().map_err(|e| ApiError::Internal(e.into()))?;
    let service = if services.len() == 1 { format!("service:{}", services[0]) } else { format!("service:({})", services.join(" OR ")) };
    Ok(Json(FleetAgentDetail {
        logs_query: format!("{service} host:{}", row.agent.hostname),
        agent: row.agent,
        checks: row.checks,
        service_checks: row.service_checks,
        config_yaml,
        full_config_reported,
        errors: errors.into_iter().map(|l| FleetLogLine { timestamp: l.ts, status: l.status, message: l.message }).collect(),
    }))
}

// ================================================================ configuration profiles

#[derive(Deserialize, ToSchema)]
pub struct FleetConfigurationsRequest {
    pub range: TimeRange,
}

/// Agents grouped by identical integrations and settings (sanitised).
#[utoipa::path(post, path = "/api/v1/fleet/configurations", operation_id = "fleet_configurations", request_body = FleetConfigurationsRequest, responses((status = 200, body = Vec<FleetProfile>)), tag = "fleet")]
pub async fn configurations(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<FleetConfigurationsRequest>) -> ApiResult<Vec<FleetProfile>> {
    let (rows, _) = load_fleet(&st, &org, req.range).await?;
    let settings: Vec<BTreeMap<String, String>> = rows
        .iter()
        .map(|r| match r.agent.kind {
            FleetAgentKind::DatadogAgent => profile_settings(r.config.as_ref()),
            // Collectors don't report their configuration: they are grouped by how they were deployed.
            FleetAgentKind::OtelCollector => BTreeMap::from([("install_method".to_string(), r.agent.install_method.clone().unwrap_or_else(|| "unknown".into()))]),
        })
        .collect();
    let pairs: Vec<(&FleetAgent, &BTreeMap<String, String>)> = rows.iter().map(|r| &r.agent).zip(settings.iter()).collect();
    Ok(Json(group_profiles(&pairs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically_and_releases_beat_their_candidates() {
        let v = |s| parse_version(s).unwrap();
        assert!(v("7.62.10") > v("7.62.9"));
        assert!(v("7.63.0") > v("7.63.0-rc.3"));
        assert!(v("7.63.0-rc.10") > v("7.63.0-rc.9"));
        // A package revision is not a pre-release; build metadata is ignored.
        assert_eq!(v("7.58.2-1"), v("7.58.2"));
        assert_eq!(v("7.60.0+git.12.abc"), v("7.60.0"));
        assert_eq!(v("otelcol-contrib 0.163.2"), v("v0.163.2"));
        assert!(parse_version("unknown").is_none());
        assert!(parse_version("7").is_none());
        assert!(parse_version("").is_none());
    }

    #[test]
    fn the_latest_version_is_the_newest_release() {
        assert_eq!(latest_version(["7.58.2", "7.62.2", "7.61.0", "7.63.0-rc.1", "garbage"]).as_deref(), Some("7.62.2"));
        // Only candidates known: the newest of them.
        assert_eq!(latest_version(["7.63.0-rc.1", "7.63.0-rc.2"]).as_deref(), Some("7.63.0-rc.2"));
        assert_eq!(latest_version(["otelcol-contrib 0.163.2", "otelcol-contrib 0.160.0"]).as_deref(), Some("0.163.2"));
        assert_eq!(latest_version(Vec::<&str>::new()), None);
    }

    #[test]
    fn outdated_means_older_than_the_latest() {
        assert!(is_outdated("7.58.2", "7.62.2"));
        assert!(!is_outdated("7.62.2", "7.62.2"));
        assert!(!is_outdated("7.63.0-rc.1", "7.62.2"));
        assert!(is_outdated("7.62.2-rc.1", "7.62.2"));
        // Unknown versions are never reported as outdated.
        assert!(!is_outdated("unknown", "7.62.2"));
    }

    #[test]
    fn secrets_are_redacted_from_configuration_values() {
        let cfg = json!({
            "api_key": "0123456789abcdef0123456789abcdef",
            "DD_APP_KEY": "abc",
            "logs_config": {"use_http": true, "auth_token": "t0k3n"},
            "proxy": {"https": "http://svc:hunter2@proxy.example.com:3128", "no_proxy": ["169.254.169.254"]},
            "instances": [{"host": "localhost", "password": "pg-pass"}, {"host": "db", "password": ""}],
            "secret_backend_timeout": 30,
            "use_token": true,
            "site": "datadoghq.example.com"
        });
        let out = sanitize(&cfg);
        assert_eq!(out["api_key"], REDACTED);
        assert_eq!(out["DD_APP_KEY"], REDACTED);
        assert_eq!(out["logs_config"]["auth_token"], REDACTED);
        assert_eq!(out["logs_config"]["use_http"], true);
        assert_eq!(out["proxy"]["https"], "http://<redacted>@proxy.example.com:3128");
        assert_eq!(out["instances"][0]["password"], REDACTED);
        assert_eq!(out["instances"][0]["host"], "localhost");
        // Empty values and booleans hold no secret.
        assert_eq!(out["instances"][1]["password"], "");
        assert_eq!(out["use_token"], true);
        // Over-redacting a number under a secret-looking key is fine.
        assert_eq!(out["secret_backend_timeout"], REDACTED);
        assert_eq!(out["site"], "datadoghq.example.com");
        assert!(!out.to_string().contains("hunter2") && !out.to_string().contains("pg-pass") && !out.to_string().contains("0123456789abcdef"));
    }

    #[test]
    fn urls_lose_their_credentials_only() {
        assert_eq!(redact_url_credentials("http://proxy.example.com:3128/x"), "http://proxy.example.com:3128/x");
        assert_eq!(redact_url_credentials("a https://u:p@h/x and http://u2@h2"), "a https://<redacted>@h/x and http://<redacted>@h2");
        assert_eq!(redact_url_credentials("mailto:ops@example.com"), "mailto:ops@example.com");
    }

    #[test]
    fn configuration_text_is_parsed_or_redacted_line_by_line() {
        let yaml = "api_key: '***************************abcde'\nsite: datadoghq.example.com\nlogs_enabled: true\nproxy:\n  https: http://u:p@proxy:3128\n";
        let v = sanitize_config_text(yaml);
        assert_eq!(v["api_key"], REDACTED);
        assert_eq!(v["logs_enabled"], true);
        assert_eq!(v["proxy"]["https"], "http://<redacted>@proxy:3128");
        // Not a mapping (a fragment, environment variables): redacted per line.
        let text = "DD_API_KEY=abc123\n  - password: hunter2\nDD_SITE=datadoghq.example.com\n  token:\n";
        let Value::String(out) = sanitize_config_text(text) else { panic!("text") };
        assert_eq!(out, "DD_API_KEY=<redacted>\n  - password: <redacted>\nDD_SITE=datadoghq.example.com\n  token:");
    }

    fn checks(runs: &[(&str, &str, &str, &str)]) -> Vec<FleetCheck> {
        summarize_checks(&runs.iter().map(|(n, i, s, m)| StoredRun { name: n.to_string(), instance: i.to_string(), status: s.to_string(), message: m.to_string() }).collect::<Vec<_>>())
    }

    #[test]
    fn checks_keep_their_worst_instance() {
        let c = checks(&[("redisdb", "redisdb:1", "OK", ""), ("redisdb", "redisdb:2", "ERROR", "connection refused"), ("cpu", "cpu", "OK", ""), ("postgres", "initialization", "ERROR", "")]);
        assert_eq!(c.len(), 3);
        let redis = c.iter().find(|c| c.name == "redisdb").unwrap();
        assert_eq!((redis.status, redis.instances, redis.message.as_deref()), (FleetCheckStatus::Error, 2, Some("connection refused")));
        assert!(c.iter().find(|c| c.name == "postgres").unwrap().load_error);
        assert_eq!(c.iter().find(|c| c.name == "cpu").unwrap().status, FleetCheckStatus::Ok);
    }

    const NOW: i64 = 1_760_000_000_000;
    const MIN: i64 = 60_000;

    #[test]
    fn an_agent_sending_metrics_with_passing_checks_is_ok() {
        let c = checks(&[("cpu", "cpu", "OK", "")]);
        let svc = vec![FleetServiceCheck { name: "datadog.agent.up".into(), status: "OK".into() }];
        assert_eq!(agent_status(NOW, Some(NOW - 20 * MIN), Some(NOW - MIN), &c, &svc), (FleetStatus::Ok, vec![]));
        // A collector has no metadata: metrics are enough.
        assert_eq!(agent_status(NOW, None, Some(NOW - 2 * MIN), &[], &[]).0, FleetStatus::Ok);
    }

    #[test]
    fn telemetry_issues_are_explained() {
        // Metadata arrives but the metrics stopped.
        let (status, issues) = agent_status(NOW, Some(NOW - 5 * MIN), Some(NOW - 40 * MIN), &[], &[]);
        assert_eq!(status, FleetStatus::Issues);
        assert_eq!(issues, vec!["Metadata still arrives, but no metrics in the last 10 minutes"]);
        let (status, issues) = agent_status(NOW, Some(NOW - 5 * MIN), None, &[], &[]);
        assert_eq!((status, issues.len()), (FleetStatus::Issues, 1));
        // Failing checks and service checks.
        let c = checks(&[("redisdb", "redisdb:1", "ERROR", "connection refused"), ("postgres", "initialization", "ERROR", ""), ("nginx", "nginx:1", "WARNING", "")]);
        let svc = vec![FleetServiceCheck { name: "datadog.agent.up".into(), status: "CRITICAL".into() }, FleetServiceCheck { name: "ntp.in_sync".into(), status: "UNKNOWN".into() }];
        let (status, issues) = agent_status(NOW, Some(NOW), Some(NOW), &c, &svc);
        assert_eq!(status, FleetStatus::Issues);
        assert_eq!(issues, vec!["Check nginx is in WARNING", "Check postgres failed to load", "Check redisdb is in ERROR: connection refused", "Service check datadog.agent.up is CRITICAL"]);
    }

    #[test]
    fn an_agent_without_recent_metrics_or_metadata_is_offline() {
        let (status, _) = agent_status(NOW, Some(NOW - 3 * 60 * MIN), Some(NOW - 3 * 60 * MIN), &[], &[]);
        assert_eq!(status, FleetStatus::Offline);
        assert_eq!(agent_status(NOW, None, Some(NOW - 11 * MIN), &[], &[]).0, FleetStatus::Offline);
        assert_eq!(agent_status(NOW, None, None, &[], &[]).0, FleetStatus::Offline);
    }

    #[test]
    fn coverage_counts_hosts_with_an_agent() {
        let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        // Four hosts in metrics (one only through a cloud integration); three run an agent, one of
        // which sends no host metrics yet.
        let c = coverage(&set(&["a", "b", "c", "cloud-only"]), &set(&["a", "b", "d"]));
        assert_eq!(c, FleetCoverage { hosts: 5, covered: 3, pct: Some(60.0) });
        assert_eq!(coverage(&set(&["a", "b", "c"]), &set(&["a"])).pct, Some(33.3));
        assert_eq!(coverage(&BTreeSet::new(), &BTreeSet::new()), FleetCoverage { hosts: 0, covered: 0, pct: None });
    }

    fn agent(host: &str, kind: FleetAgentKind, version: &str, integrations: &[&str]) -> FleetAgent {
        FleetAgent {
            id: format!("{}:{host}", super::super::hosts::enum_str(&kind)),
            hostname: host.into(),
            kind,
            version: Some(version.into()),
            outdated: false,
            os: Some("linux".into()),
            platform: Some("ubuntu 24.04".into()),
            status: FleetStatus::Ok,
            issues: vec![],
            integrations: integrations.iter().map(|s| s.to_string()).collect(),
            env: Some("prod".into()),
            cluster: Some("shop-prod-eu".into()),
            last_reported: Some(NOW),
            install_method: Some("helm".into()),
        }
    }

    #[test]
    fn the_configuration_document_is_sanitised_and_profiles_ignore_the_host() {
        let payload = json!({"agentVersion": "7.62.2", "agent-flavor": "agent", "install-method": {"tool": "helm", "tool_version": "3.14"}, "logs": {"transport": "HTTP"}, "python": "3.12.6",
            "otlp": {"enabled": false}, "proxy-info": {"no-proxy-nonexact-match": false}, "systemStats": {}, "host-tags": {"system": ["env:prod"]}});
        let inventory = json!({"feature_logs_enabled": true, "config_site": "datadoghq.example.com", "config_proxy_https": "http://u:p@proxy:3128", "hostname_source": "os",
            "full_configuration": "api_key: abcdef\nhostname: i-0a1b2c3d4e5f60001\nlogs_enabled: true\n"});
        let integrations = vec!["redisdb".to_string(), "system".to_string()];
        let doc = config_document(Some(&payload), Some(&inventory), &integrations).unwrap();
        assert_eq!(doc["agent"]["install-method"]["tool"], "helm");
        assert!(doc["agent"].get("host-tags").is_none(), "host tags describe the host, not the agent");
        assert_eq!(doc["inventory"]["config_proxy_https"], "http://<redacted>@proxy:3128");
        assert!(doc["inventory"].get("hostname_source").is_none());
        assert_eq!(doc["configuration"]["api_key"], REDACTED);
        assert_eq!(doc["integrations"], json!(["redisdb", "system"]));
        let settings = profile_settings(Some(&doc));
        assert_eq!(settings.get("install-method.tool").map(String::as_str), Some("helm"));
        assert_eq!(settings.get("feature_logs_enabled").map(String::as_str), Some("true"));
        assert!(!settings.keys().any(|k| k.contains("hostname")), "{settings:?}");
        // Versions of the Agent's parts change with upgrades; they aren't configuration.
        assert!(!settings.contains_key("install-method.tool_version") && !settings.contains_key("python"), "{settings:?}");
        assert!(config_document(None, None, &[]).is_none());

        // Two agents with the same setup and integrations share a profile, whatever their host and version.
        let a = agent("i-0a1b2c3d4e5f60001", FleetAgentKind::DatadogAgent, "7.62.2", &["redisdb", "system"]);
        let b = agent("i-0a1b2c3d4e5f60002", FleetAgentKind::DatadogAgent, "7.58.1", &["redisdb", "system"]);
        let c = agent("i-0a1b2c3d4e5f60003", FleetAgentKind::DatadogAgent, "7.62.2", &["system"]);
        let d = agent("i-0a1b2c3d4e5f60004", FleetAgentKind::OtelCollector, "0.163.2", &[]);
        let empty = BTreeMap::new();
        let profiles = group_profiles(&[(&a, &settings), (&b, &settings), (&c, &settings), (&d, &empty)]);
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[0].agents.iter().map(|a| a.hostname.as_str()).collect::<Vec<_>>(), vec!["i-0a1b2c3d4e5f60001", "i-0a1b2c3d4e5f60002"]);
        assert_eq!(profiles[0].versions, vec!["7.62.2", "7.58.1"]);
        assert_eq!(profiles[0].id.len(), 8);
        let again = group_profiles(&[(&b, &settings), (&a, &settings)]);
        assert_eq!(again[0].id, profiles[0].id, "profile ids are stable");
    }

    #[test]
    fn search_and_facets() {
        let mut a = agent("i-0a1b2c3d4e5f60001", FleetAgentKind::DatadogAgent, "7.62.2", &["redisdb", "system"]);
        let mut b = agent("i-0a1b2c3d4e5f60002", FleetAgentKind::DatadogAgent, "7.58.1", &["system"]);
        b.outdated = true;
        b.status = FleetStatus::Issues;
        a.cluster = Some("shop-staging".into());
        let c = agent("i-0a1b2c3d4e5f60003", FleetAgentKind::OtelCollector, "0.163.2", &[]);
        let all = [&a, &b, &c];
        let hits = |q: &str| all.iter().filter(|x| agent_matches(x, &parse_query(q))).map(|x| x.hostname.clone()).collect::<Vec<_>>();
        assert_eq!(hits("integration:redisdb"), vec!["i-0a1b2c3d4e5f60001"]);
        assert_eq!(hits("-integration:redisdb type:datadog_agent"), vec!["i-0a1b2c3d4e5f60002"]);
        assert_eq!(hits("status:issues outdated:true"), vec!["i-0a1b2c3d4e5f60002"]);
        assert_eq!(hits("type:otel_collector"), vec!["i-0a1b2c3d4e5f60003"]);
        assert_eq!(hits("cluster:shop-prod-eu version:7.58.1"), vec!["i-0a1b2c3d4e5f60002"]);
        assert_eq!(hits("60003").len(), 1, "free text on the hostname");
        let f = facets(&all);
        let integration = f.iter().find(|f| f.key == "integration").unwrap();
        assert_eq!(integration.values.iter().map(|v| (v.value.as_str(), v.count)).collect::<Vec<_>>(), vec![("system", 2), ("redisdb", 1)]);
        let v = versions(&all);
        assert_eq!(v.iter().map(|v| (v.version.as_deref().unwrap(), v.count, v.outdated)).collect::<Vec<_>>(), vec![("7.62.2", 1, false), ("7.58.1", 1, true), ("0.163.2", 1, false)]);
    }
}
