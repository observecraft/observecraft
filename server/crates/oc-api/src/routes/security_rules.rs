//! Cloud SIEM detection rules (Security → Cloud SIEM → Detection Rules): Datadog's security
//! monitoring rules, in Datadog's own document shape (camelCase, as `/api/v2/security_monitoring/rules`
//! sends them), so the UI and the Terraform provider ([`super::datadog_api::security_monitoring`])
//! read and write the same records.
//!
//! - A rule has queries (`a`, `b`, … by position unless named) over the org's logs, cases (a
//!   condition on the queries' values and the signal's severity) and options (evaluation window,
//!   keep-alive, maximum signal duration). [`super::security_engine`] evaluates the `log_detection`
//!   rules with the `threshold` method and the `count` and `cardinality` aggregations; the other
//!   types and methods are stored, listed and shown as not evaluated.
//! - Default rules come from the content packs ([`super::security_catalog`]): every org has them,
//!   on while their pack is on. They can be turned on and off, not changed or deleted.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::security_catalog::{default_rules, pack};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const SEVERITIES: [&str; 5] = ["info", "low", "medium", "high", "critical"];
pub const RULE_TYPES: [&str; 7] = ["log_detection", "workload_security", "application_security", "api_security", "signal_correlation", "cloud_configuration", "infrastructure_configuration"];
pub const DETECTION_METHODS: [&str; 8] = ["threshold", "new_value", "anomaly_detection", "impossible_travel", "hardcoded", "third_party", "anomaly_threshold", "sequence_detection"];
pub const AGGREGATIONS: [&str; 8] = ["count", "cardinality", "sum", "max", "new_value", "geo_data", "event_count", "none"];

/// Durations Datadog accepts, in seconds.
pub const EVALUATION_WINDOWS: &[i64] = &[0, 60, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400];
pub const KEEP_ALIVES: &[i64] = &[0, 60, 300, 600, 900, 1800, 3600, 7200, 10800, 21600];
pub const MAX_DURATIONS: &[i64] = &[0, 60, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400];

fn option_label(k: &str) -> &'static str {
    match k {
        "evaluationWindow" => "Evaluation window",
        "keepAlive" => "Keep alive",
        _ => "Max signal duration",
    }
}

pub fn severity_rank(s: &str) -> usize {
    SEVERITIES.iter().position(|x| *x == s).unwrap_or(0)
}

// ================================================================ conditions

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
struct Cmp {
    var: String,
    op: Op,
    value: f64,
}

/// A case's condition: comparisons of query values joined with `&&` and `||` (`a > 5 && b > 0`).
/// Empty means always.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition(Vec<Vec<Cmp>>);

impl Condition {
    pub fn parse(s: &str) -> Result<Condition, String> {
        if s.trim().is_empty() {
            return Ok(Condition(vec![]));
        }
        let mut ors = Vec::new();
        for part in s.split("||") {
            let mut ands = Vec::new();
            for c in part.split("&&") {
                let c = c.trim().trim_start_matches('(').trim_end_matches(')').trim();
                let (pos, op, len) = [(">=", Op::Ge), ("<=", Op::Le), ("==", Op::Eq), ("!=", Op::Ne), (">", Op::Gt), ("<", Op::Lt)]
                    .iter()
                    .find_map(|(t, op)| c.find(t).map(|p| (p, *op, t.len())))
                    .ok_or_else(|| format!("invalid condition `{c}`: expected a comparison such as `a > 0`"))?;
                let var = c[..pos].trim();
                let value = c[pos + len..].trim();
                if var.is_empty() || !var.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                    return Err(format!("invalid condition `{c}`: `{var}` is not a query name"));
                }
                let value: f64 = value.parse().map_err(|_| format!("invalid condition `{c}`: `{value}` is not a number"))?;
                ands.push(Cmp { var: var.to_string(), op, value });
            }
            ors.push(ands);
        }
        Ok(Condition(ors))
    }

    pub fn vars(&self) -> impl Iterator<Item = &str> {
        self.0.iter().flatten().map(|c| c.var.as_str())
    }

    pub fn eval(&self, values: &HashMap<String, f64>) -> bool {
        if self.0.is_empty() {
            return true;
        }
        self.0.iter().any(|ands| {
            ands.iter().all(|c| {
                let v = values.get(&c.var).copied().unwrap_or(0.0);
                match c.op {
                    Op::Gt => v > c.value,
                    Op::Ge => v >= c.value,
                    Op::Lt => v < c.value,
                    Op::Le => v <= c.value,
                    Op::Eq => v == c.value,
                    Op::Ne => v != c.value,
                }
            })
        })
    }
}

// ================================================================ model

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleQuery {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub group_by_fields: Vec<String>,
    #[serde(default)]
    pub distinct_fields: Vec<String>,
    #[serde(default = "default_aggregation")]
    pub aggregation: String,
    /// The name conditions use; empty: `a`, `b`, … by position.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub has_optional_group_by_fields: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
}

fn default_aggregation() -> String {
    "count".into()
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleCase {
    #[serde(default)]
    pub name: String,
    /// info, low, medium, high or critical.
    pub status: String,
    #[serde(default)]
    pub condition: String,
    #[serde(default)]
    pub notifications: Vec<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleFilter {
    /// require or suppress.
    pub action: String,
    #[serde(default)]
    pub query: String,
}

/// A detection rule as Datadog sends it, plus what the UI shows about its evaluation.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRule {
    pub id: String,
    pub version: i32,
    pub name: String,
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_enabled: bool,
    pub is_default: bool,
    pub queries: Vec<RuleQuery>,
    pub cases: Vec<RuleCase>,
    /// evaluationWindow, keepAlive, maxSignalDuration (seconds), detectionMethod, …
    #[schema(value_type = Object)]
    pub options: Value,
    pub filters: Vec<RuleFilter>,
    pub tags: Vec<String>,
    pub has_extended_title: bool,
    /// Epoch ms.
    pub created_at: i64,
    pub updated_at: i64,
    pub creation_author_id: i64,
    pub update_author_id: i64,
    /// The content pack of a default rule.
    pub pack: Option<String>,
    /// Why the engine doesn't evaluate the rule (its type, method or aggregation); absent when it does.
    pub not_evaluated: Option<String>,
    /// Whether the rule raises signals now: enabled, and its pack on.
    pub active: bool,
    /// Kept for read-back, not interpreted (referenceTables, groupSignalsBy, schedulingOptions, …).
    #[serde(skip)]
    pub extra: Value,
}

/// A rule as the UI creates or changes it (the same fields as Datadog's create body).
#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRuleInput {
    pub name: String,
    #[serde(default)]
    pub message: String,
    #[serde(rename = "type", default = "default_type")]
    pub kind: String,
    #[serde(default = "yes")]
    pub is_enabled: bool,
    pub queries: Vec<RuleQuery>,
    pub cases: Vec<RuleCase>,
    #[serde(default = "empty_object")]
    #[schema(value_type = Object)]
    pub options: Value,
    #[serde(default)]
    pub filters: Vec<RuleFilter>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub has_extended_title: bool,
}

fn default_type() -> String {
    "log_detection".into()
}
fn yes() -> bool {
    true
}
fn empty_object() -> Value {
    json!({})
}

/// The query names conditions refer to, by position.
pub fn query_names(queries: &[RuleQuery]) -> Vec<String> {
    queries.iter().enumerate().map(|(i, q)| if q.name.is_empty() { ((b'a' + i as u8) as char).to_string() } else { q.name.clone() }).collect()
}

pub fn option_i64(options: &Value, key: &str, default: i64) -> i64 {
    options.get(key).and_then(Value::as_i64).unwrap_or(default)
}

pub fn detection_method(options: &Value) -> &str {
    options.get("detectionMethod").and_then(Value::as_str).unwrap_or("threshold")
}

/// Fills Datadog's option defaults (threshold, 5 min window, 1 h keep-alive, 1 day max).
pub fn normalize_options(mut options: Value) -> Value {
    if !options.is_object() {
        options = json!({});
    }
    let o = options.as_object_mut().unwrap();
    o.entry("detectionMethod").or_insert(json!("threshold"));
    o.entry("evaluationWindow").or_insert(json!(300));
    o.entry("keepAlive").or_insert(json!(3600));
    o.entry("maxSignalDuration").or_insert(json!(86400));
    options
}

impl SecurityRuleInput {
    /// Datadog's checks of a rule body, as `400` messages.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("the rule needs a name".into());
        }
        if !RULE_TYPES.contains(&self.kind.as_str()) {
            return Err(format!("unknown rule type `{}`", self.kind));
        }
        if self.queries.is_empty() && self.kind != "cloud_configuration" {
            return Err("the rule needs at least one query".into());
        }
        if self.queries.len() > 10 {
            return Err("a rule has at most 10 queries".into());
        }
        for q in &self.queries {
            if q.query.trim().is_empty() && matches!(self.kind.as_str(), "log_detection" | "workload_security" | "application_security" | "api_security") {
                return Err("Query filter cannot be empty".into());
            }
            if !AGGREGATIONS.contains(&q.aggregation.as_str()) {
                return Err(format!("unknown aggregation `{}`", q.aggregation));
            }
            if q.aggregation == "cardinality" && q.distinct_fields.is_empty() {
                return Err("a cardinality query needs a distinct field".into());
            }
        }
        let method = detection_method(&self.options);
        if !DETECTION_METHODS.contains(&method) {
            return Err(format!("unknown detection method `{method}`"));
        }
        if self.cases.is_empty() && !matches!(method, "new_value" | "anomaly_detection" | "impossible_travel" | "third_party") {
            return Err("the rule needs at least one case".into());
        }
        let names = query_names(&self.queries);
        for c in &self.cases {
            if !SEVERITIES.contains(&c.status.as_str()) {
                return Err(format!("invalid case status `{}`: use info, low, medium, high or critical", c.status));
            }
            let cond = Condition::parse(&c.condition)?;
            let unknown = cond.vars().find(|v| !names.iter().any(|n| n == v)).map(str::to_string);
            if let Some(v) = unknown {
                return Err(format!("the condition `{}` uses `{v}`, which is not a query of the rule", c.condition));
            }
        }
        for f in &self.filters {
            if !matches!(f.action.as_str(), "require" | "suppress") {
                return Err(format!("invalid filter action `{}`: use require or suppress", f.action));
            }
        }
        for (k, allowed) in [("evaluationWindow", EVALUATION_WINDOWS), ("keepAlive", KEEP_ALIVES), ("maxSignalDuration", MAX_DURATIONS)] {
            if let Some(v) = self.options.get(k) {
                if !v.as_i64().is_some_and(|n| allowed.contains(&n)) {
                    let minutes: Vec<String> = allowed.iter().map(|s| (s / 60).to_string()).collect();
                    return Err(format!("{} is not in allowed durations: {} (in minutes)", option_label(k), minutes.join(", ")));
                }
            }
        }
        let keep = option_i64(&self.options, "keepAlive", 3600);
        let max = option_i64(&self.options, "maxSignalDuration", 86400);
        if self.options.get("maxSignalDuration").is_some() && max < keep {
            return Err("Max signal duration must be greater than or equal to keep alive".into());
        }
        Ok(())
    }
}

/// Why the engine skips a rule, if it does.
pub fn not_evaluated(kind: &str, queries: &[RuleQuery], options: &Value) -> Option<String> {
    if !matches!(kind, "log_detection" | "application_security" | "workload_security") {
        return Some(format!("{} rules are stored but not evaluated yet", kind.replace('_', " ")));
    }
    let method = detection_method(options);
    if method != "threshold" {
        return Some(format!("the {} detection method is not evaluated yet", method.replace('_', " ")));
    }
    let counts_only = kind != "log_detection";
    if let Some(q) = queries.iter().find(|q| !matches!(q.aggregation.as_str(), "count" | "cardinality") || (counts_only && q.aggregation != "count")) {
        return Some(format!("the {} aggregation is not evaluated yet", q.aggregation));
    }
    None
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: String,
    version: i32,
    name: String,
    message: String,
    #[sqlx(rename = "type")]
    kind: String,
    is_enabled: bool,
    is_default: bool,
    pack: Option<String>,
    queries: Value,
    cases: Value,
    options: Value,
    filters: Value,
    tags: Vec<String>,
    has_extended_title: bool,
    extra: Value,
    creation_author: i64,
    update_author: i64,
    created_at: i64,
    updated_at: i64,
    pack_enabled: bool,
}

const SELECT: &str = "SELECT r.id, r.version, r.name, r.message, r.type, r.is_enabled, r.is_default, r.pack, r.queries, r.cases, r.options, r.filters, r.tags, \
    r.has_extended_title, r.extra, r.creation_author, r.update_author, (extract(epoch FROM r.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM r.updated_at) * 1000)::bigint AS updated_at, coalesce(p.enabled, true) AS pack_enabled \
    FROM security_rules r LEFT JOIN security_content_packs p ON p.org_id = r.org_id AND p.pack = r.pack";

impl From<RuleRow> for SecurityRule {
    fn from(r: RuleRow) -> Self {
        let queries: Vec<RuleQuery> = serde_json::from_value(r.queries).unwrap_or_default();
        let options = normalize_options(r.options);
        SecurityRule {
            not_evaluated: not_evaluated(&r.kind, &queries, &options),
            active: r.is_enabled && r.pack_enabled,
            id: r.id,
            version: r.version,
            name: r.name,
            message: r.message,
            kind: r.kind,
            is_enabled: r.is_enabled,
            is_default: r.is_default,
            queries,
            cases: serde_json::from_value(r.cases).unwrap_or_default(),
            options,
            filters: serde_json::from_value(r.filters).unwrap_or_default(),
            tags: r.tags,
            has_extended_title: r.has_extended_title,
            created_at: r.created_at,
            updated_at: r.updated_at,
            creation_author_id: r.creation_author,
            update_author_id: r.update_author,
            pack: r.pack,
            extra: r.extra,
        }
    }
}

pub async fn load_rules(st: &AppState, org: &str) -> Result<Vec<SecurityRule>, ApiError> {
    ensure_defaults(st, org).await?;
    let rows: Vec<RuleRow> = sqlx::query_as(&format!("{SELECT} WHERE r.org_id = $1 ORDER BY r.is_default, r.name")).bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(SecurityRule::from).collect())
}

pub async fn load_rule(st: &AppState, org: &str, id: &str) -> Result<SecurityRule, ApiError> {
    ensure_defaults(st, org).await?;
    let row: Option<RuleRow> = sqlx::query_as(&format!("{SELECT} WHERE r.org_id = $1 AND r.id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(SecurityRule::from).ok_or_else(|| ApiError::NotFound(format!("no detection rule {id}")))
}

/// A Datadog-style id: three groups of three lower-case letters and digits.
pub fn new_id() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..3).map(|_| (0..3).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect::<String>()).collect::<Vec<_>>().join("-")
}

/// A number for a user (Datadog's author ids are numbers).
pub fn author(ctx: &AuthContext) -> i64 {
    super::datadog_api::monitors::org_number(&ctx.user_id) & 0x7fff_ffff_ffff
}

pub async fn insert_rule(st: &AppState, ctx: &AuthContext, input: &SecurityRuleInput, extra: Value) -> Result<String, ApiError> {
    input.validate().map_err(ApiError::BadRequest)?;
    let id = new_id();
    sqlx::query(
        "INSERT INTO security_rules (org_id, id, name, message, type, is_enabled, queries, cases, options, filters, tags, has_extended_title, extra, creation_author, update_author) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(input.name.trim())
    .bind(&input.message)
    .bind(&input.kind)
    .bind(input.is_enabled)
    .bind(json!(input.queries))
    .bind(json!(input.cases))
    .bind(normalize_options(input.options.clone()))
    .bind(json!(input.filters))
    .bind(&input.tags)
    .bind(input.has_extended_title)
    .bind(extra)
    .bind(author(ctx))
    .execute(&st.postgres)
    .await?;
    Ok(id)
}

pub async fn update_rule_row(st: &AppState, ctx: &AuthContext, id: &str, input: &SecurityRuleInput, extra: Value) -> Result<(), ApiError> {
    let current = load_rule(st, &ctx.org_id, id).await?;
    if current.is_default {
        // A default rule's definition belongs to its pack: only turning it on and off.
        sqlx::query("UPDATE security_rules SET is_enabled = $3, updated_at = now(), update_author = $4 WHERE org_id = $1 AND id = $2")
            .bind(&ctx.org_id)
            .bind(id)
            .bind(input.is_enabled)
            .bind(author(ctx))
            .execute(&st.postgres)
            .await?;
        return Ok(());
    }
    input.validate().map_err(ApiError::BadRequest)?;
    sqlx::query(
        "UPDATE security_rules SET name = $3, message = $4, type = $5, is_enabled = $6, queries = $7, cases = $8, options = $9, filters = $10, tags = $11, \
         has_extended_title = $12, extra = $13, update_author = $14, updated_at = now(), version = version + 1 WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(input.name.trim())
    .bind(&input.message)
    .bind(&input.kind)
    .bind(input.is_enabled)
    .bind(json!(input.queries))
    .bind(json!(input.cases))
    .bind(normalize_options(input.options.clone()))
    .bind(json!(input.filters))
    .bind(&input.tags)
    .bind(input.has_extended_title)
    .bind(extra)
    .bind(author(ctx))
    .execute(&st.postgres)
    .await?;
    Ok(())
}

pub async fn delete_rule_row(st: &AppState, org: &str, id: &str) -> Result<(), ApiError> {
    let current = load_rule(st, org, id).await?;
    if current.is_default {
        return Err(ApiError::BadRequest("default rules can't be deleted; disable the rule instead".into()));
    }
    sqlx::query("DELETE FROM security_rules WHERE org_id = $1 AND id = $2").bind(org).bind(id).execute(&st.postgres).await?;
    Ok(())
}

fn defaults_installed() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

/// Installs the content packs' default rules for an org (once per process; new catalog versions
/// replace older definitions, keeping whether each rule is on).
pub async fn ensure_defaults(st: &AppState, org: &str) -> Result<(), ApiError> {
    if defaults_installed().lock().unwrap().contains(org) {
        return Ok(());
    }
    for (p, r) in default_rules() {
        let names = ["a"];
        let queries = json!([{ "query": r.query, "groupByFields": r.group_by, "distinctFields": [], "aggregation": "count", "name": names[0], "hasOptionalGroupByFields": false }]);
        let cases: Vec<Value> = r.cases.iter().map(|(cond, sev, name)| json!({ "name": name, "status": sev, "condition": cond, "notifications": [] })).collect();
        let mut tags: Vec<String> = r.tags.iter().map(|t| t.to_string()).collect();
        tags.push(format!("source:{}", p.query.trim_start_matches("source:")));
        let options = json!({ "detectionMethod": "threshold", "evaluationWindow": r.window, "keepAlive": 3600, "maxSignalDuration": 86400 });
        sqlx::query(
            "INSERT INTO security_rules (org_id, id, version, name, message, type, is_enabled, is_default, pack, default_rule_id, queries, cases, options, tags, has_extended_title) \
             VALUES ($1, $2, $3, $4, $5, 'log_detection', true, true, $6, $2, $7, $8, $9, $10, true) \
             ON CONFLICT (org_id, id) DO UPDATE SET version = EXCLUDED.version, name = EXCLUDED.name, message = EXCLUDED.message, queries = EXCLUDED.queries, \
             cases = EXCLUDED.cases, options = EXCLUDED.options, tags = EXCLUDED.tags, updated_at = now() \
             WHERE security_rules.is_default AND security_rules.version < EXCLUDED.version",
        )
        .bind(org)
        .bind(r.id)
        .bind(r.version)
        .bind(r.name)
        .bind(r.message)
        .bind(p.id)
        .bind(queries)
        .bind(json!(cases))
        .bind(options)
        .bind(&tags)
        .execute(&st.postgres)
        .await?;
    }
    // App & API Protection's default rules, on the security traces.
    for (id, name, message, query, group_by, cases, window) in super::appsec::DEFAULT_RULES {
        let queries = json!([{ "query": query, "groupByFields": group_by, "distinctFields": [], "aggregation": "count", "name": "a", "hasOptionalGroupByFields": false }]);
        let cases: Vec<Value> = cases.iter().map(|(cond, sev, name)| json!({ "name": name, "status": sev, "condition": cond, "notifications": [] })).collect();
        let options = json!({ "detectionMethod": "threshold", "evaluationWindow": window, "keepAlive": 3600, "maxSignalDuration": 86400 });
        sqlx::query(
            "INSERT INTO security_rules (org_id, id, version, name, message, type, is_enabled, is_default, pack, default_rule_id, queries, cases, options, tags, has_extended_title) \
             VALUES ($1, $2, 1, $3, $4, 'application_security', true, true, 'app-sec', $2, $5, $6, $7, $8, true) ON CONFLICT (org_id, id) DO NOTHING",
        )
        .bind(org)
        .bind(id)
        .bind(name)
        .bind(message)
        .bind(queries)
        .bind(json!(cases))
        .bind(options)
        .bind(vec!["source:application-security".to_string(), "category:attack_attempt".to_string()])
        .execute(&st.postgres)
        .await?;
    }
    // AI Guard's default rules, on the evaluations.
    for (id, name, message, query, group_by, cases, window, rule_tags) in super::ai_guard::DEFAULT_RULES {
        let queries = json!([{ "query": query, "groupByFields": group_by, "distinctFields": [], "aggregation": "count", "name": "a", "hasOptionalGroupByFields": false }]);
        let cases: Vec<Value> = cases.iter().map(|(cond, sev, name)| json!({ "name": name, "status": sev, "condition": cond, "notifications": [] })).collect();
        let options = json!({ "detectionMethod": "threshold", "evaluationWindow": window, "keepAlive": 3600, "maxSignalDuration": 86400 });
        let mut tags: Vec<String> = vec!["source:application-threats".into(), "ai_guard:true".into()];
        tags.extend(rule_tags.iter().map(|t| t.to_string()));
        sqlx::query(
            "INSERT INTO security_rules (org_id, id, version, name, message, type, is_enabled, is_default, pack, default_rule_id, queries, cases, options, tags, has_extended_title) \
             VALUES ($1, $2, 1, $3, $4, 'application_security', true, true, 'ai-guard', $2, $5, $6, $7, $8, true) ON CONFLICT (org_id, id) DO NOTHING",
        )
        .bind(org)
        .bind(id)
        .bind(name)
        .bind(message)
        .bind(queries)
        .bind(json!(cases))
        .bind(options)
        .bind(&tags)
        .execute(&st.postgres)
        .await?;
    }
    // Workload Protection's default rules, one per threat detection agent rule, on its events.
    for r in super::workload_catalog::AGENT_RULES.iter().filter(|r| r.policy == "threat-detection") {
        let id = super::workload_catalog::detection_rule_id(r.name);
        let queries = json!([{ "query": format!("@agent.rule_id:{}", r.name), "groupByFields": ["host"], "distinctFields": [], "aggregation": "count", "name": "a", "hasOptionalGroupByFields": false }]);
        let cases = json!([{ "name": "", "status": r.severity, "condition": "a > 0", "notifications": [] }]);
        let options = json!({ "detectionMethod": "threshold", "evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400 });
        let mut tags = super::workload_catalog::mitre_tags(r);
        tags.push("source:runtime-security-agent".into());
        sqlx::query(
            "INSERT INTO security_rules (org_id, id, version, name, message, type, is_enabled, is_default, pack, default_rule_id, queries, cases, options, tags, has_extended_title) \
             VALUES ($1, $2, 1, $3, $4, 'workload_security', true, true, 'workload-protection', $2, $5, $6, $7, $8, true) ON CONFLICT (org_id, id) DO NOTHING",
        )
        .bind(org)
        .bind(&id)
        .bind(r.title)
        .bind(r.description)
        .bind(queries)
        .bind(cases)
        .bind(options)
        .bind(&tags)
        .execute(&st.postgres)
        .await?;
    }
    defaults_installed().lock().unwrap().insert(org.to_string());
    Ok(())
}

// ================================================================ UI API

/// A rule in the list, with its recent signals.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRuleSummary {
    #[serde(flatten)]
    pub rule: SecurityRule,
    /// Signals raised in the last 7 days.
    pub signals7d: i64,
    /// Epoch ms of the latest signal.
    pub last_signal_at: Option<i64>,
    pub pack_name: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/security/rules", operation_id = "list_security_rules", responses((status = 200, body = Vec<SecurityRuleSummary>)), tag = "security")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<SecurityRuleSummary>> {
    let rules = load_rules(&st, &ctx.org_id).await?;
    let stats: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT rule_id, count(*) FILTER (WHERE created_at > now() - interval '7 days'), (extract(epoch FROM max(created_at)) * 1000)::bigint \
         FROM security_signals WHERE org_id = $1 GROUP BY rule_id",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    let stats: HashMap<String, (i64, Option<i64>)> = stats.into_iter().map(|(r, n, t)| (r, (n, t))).collect();
    Ok(Json(
        rules
            .into_iter()
            .map(|rule| {
                let (signals7d, last_signal_at) = stats.get(&rule.id).copied().unwrap_or((0, None));
                let pack_name = rule.pack.as_deref().and_then(pack).map(|p| p.name.to_string());
                SecurityRuleSummary { rule, signals7d, last_signal_at, pack_name }
            })
            .collect(),
    ))
}

#[utoipa::path(get, path = "/api/v1/security/rules/{id}", operation_id = "get_security_rule", params(("id" = String, Path)),
    responses((status = 200, body = SecurityRule), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn get(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<SecurityRule> {
    Ok(Json(load_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/security/rules", operation_id = "create_security_rule", request_body = SecurityRuleInput,
    responses((status = 200, body = SecurityRule), (status = 400, body = crate::error::ErrorBody)), tag = "security")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<SecurityRuleInput>) -> ApiResult<SecurityRule> {
    let id = insert_rule(&st, &ctx, &input, json!({})).await?;
    Ok(Json(load_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/security/rules/{id}", operation_id = "update_security_rule", params(("id" = String, Path)), request_body = SecurityRuleInput,
    responses((status = 200, body = SecurityRule), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SecurityRuleInput>) -> ApiResult<SecurityRule> {
    let extra = load_rule(&st, &ctx.org_id, &id).await?.extra;
    update_rule_row(&st, &ctx, &id, &input, extra).await?;
    Ok(Json(load_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/security/rules/{id}", operation_id = "delete_security_rule", params(("id" = String, Path)),
    responses((status = 204), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    delete_rule_row(&st, &ctx.org_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct RuleEnabled {
    pub enabled: bool,
}

#[utoipa::path(put, path = "/api/v1/security/rules/{id}/enabled", operation_id = "enable_security_rule", params(("id" = String, Path)), request_body = RuleEnabled,
    responses((status = 200, body = SecurityRule), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn set_enabled(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(req): Json<RuleEnabled>) -> ApiResult<SecurityRule> {
    load_rule(&st, &ctx.org_id, &id).await?;
    sqlx::query("UPDATE security_rules SET is_enabled = $3, updated_at = now(), update_author = $4 WHERE org_id = $1 AND id = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(req.enabled)
        .bind(author(&ctx))
        .execute(&st.postgres)
        .await?;
    Ok(Json(load_rule(&st, &ctx.org_id, &id).await?))
}

/// A minimal rule for tests: counts `source:nginx` by service, `a > 100` → info.
#[cfg(test)]
pub fn test_rule() -> SecurityRule {
    SecurityRule {
        id: "abc-def-ghi".into(),
        version: 1,
        name: "Attack attempts".into(),
        message: String::new(),
        kind: "log_detection".into(),
        is_enabled: true,
        is_default: false,
        queries: vec![RuleQuery {
            query: "source:nginx".into(),
            group_by_fields: vec!["service".into()],
            distinct_fields: vec![],
            aggregation: "count".into(),
            name: String::new(),
            has_optional_group_by_fields: false,
            metric: None,
            metrics: None,
            data_source: None,
        }],
        cases: vec![RuleCase { name: "Condition 1".into(), status: "info".into(), condition: "a > 100".into(), notifications: vec![] }],
        options: json!({}),
        filters: vec![],
        tags: vec![],
        has_extended_title: false,
        created_at: 0,
        updated_at: 0,
        creation_author_id: 0,
        update_author_id: 0,
        pack: None,
        not_evaluated: None,
        active: true,
        extra: json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn conditions_parse_and_evaluate() {
        let c = Condition::parse("a > 5 && b >= 1 || c == 3").unwrap();
        assert!(c.eval(&vars(&[("a", 6.0), ("b", 1.0)])));
        assert!(!c.eval(&vars(&[("a", 6.0)])));
        assert!(c.eval(&vars(&[("c", 3.0)])));
        assert!(Condition::parse("").unwrap().eval(&HashMap::new()));
        assert!(Condition::parse("a>0").unwrap().eval(&vars(&[("a", 1.0)])));
        assert!(Condition::parse("a > x").is_err());
        assert!(Condition::parse("> 3").is_err());
        assert_eq!(Condition::parse("a > 0 && b > 1").unwrap().vars().collect::<Vec<_>>(), ["a", "b"]);
    }

    fn input() -> SecurityRuleInput {
        serde_json::from_value(json!({
            "name": "Brute force", "message": "m", "isEnabled": true, "type": "log_detection",
            "queries": [{"query": "source:okta", "groupByFields": ["@usr.email"], "aggregation": "count"}],
            "cases": [{"condition": "a > 5", "status": "high"}],
            "options": {"evaluationWindow": 900, "keepAlive": 3600, "maxSignalDuration": 86400}
        }))
        .unwrap()
    }

    #[test]
    fn validation_checks_cases_queries_and_options() {
        assert!(input().validate().is_ok());
        let mut i = input();
        i.cases[0].condition = "b > 5".into();
        assert!(i.validate().unwrap_err().contains("not a query"));
        let mut i = input();
        i.cases[0].status = "urgent".into();
        assert!(i.validate().is_err());
        let mut i = input();
        i.queries[0].aggregation = "cardinality".into();
        assert!(i.validate().is_err());
        let mut i = input();
        i.options = json!({"evaluationWindow": -1});
        assert!(i.validate().is_err());
        let mut i = input();
        i.options = json!({"keepAlive": 999999});
        assert!(i.validate().unwrap_err().contains("Keep alive is not in allowed durations"));
        let mut i = input();
        i.options = json!({"keepAlive": 3600, "maxSignalDuration": 1800});
        assert!(i.validate().unwrap_err().contains("greater than or equal"));
        let mut i = input();
        i.queries[0].query = String::new();
        assert_eq!(i.validate().unwrap_err(), "Query filter cannot be empty");
        let mut i = input();
        i.queries.push(RuleQuery { name: "failures".into(), ..i.queries[0].clone() });
        i.cases[0].condition = "a > 1 && failures > 2".into();
        assert!(i.validate().is_ok());
    }

    #[test]
    fn query_names_default_to_letters() {
        let i = input();
        let mut qs = i.queries.clone();
        qs.push(RuleQuery { name: "x".into(), ..qs[0].clone() });
        qs.push(qs[0].clone());
        assert_eq!(query_names(&qs), ["a", "x", "c"]);
    }

    #[test]
    fn only_threshold_log_rules_with_counts_are_evaluated() {
        let i = input();
        assert!(not_evaluated("log_detection", &i.queries, &json!({})).is_none());
        assert!(not_evaluated("workload_security", &i.queries, &json!({})).is_none());
        assert!(not_evaluated("signal_correlation", &i.queries, &json!({})).is_some());
        assert!(not_evaluated("log_detection", &i.queries, &json!({"detectionMethod": "new_value"})).is_some());
    }
}
