//! Security signals (Security → Signals, Cloud SIEM → Signals): what the detection rules raised
//! ([`super::security_engine`]), searched, faceted and triaged.
//!
//! - Search: the signals of a time range (by creation) matching a query in the search syntax:
//!   `status:` / `severity:`, `@workflow.triage.state:` (or `state:`), `source:`, `service:`, `env:`,
//!   `rule:` / `@workflow.rule.id:`, `@workflow.rule.type:`, `assignee:`, any tag of the signal
//!   (`tactic:…`), group values (`@usr.email:…`) and free text in the title.
//! - Triage: state (open, under review, archived with a reason), assignee, and the related incidents.
//! - The signal's logs are read by the UI through the logs API with the reader's access, using the
//!   `logsQuery` returned here.

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{parse_query, Term};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const STATES: [&str; 3] = ["open", "under_review", "archived"];
pub const ARCHIVE_REASONS: [&str; 7] = ["none", "false_positive", "testing_or_maintenance", "investigated_case_opened", "true_positive_benign", "true_positive_malicious", "other"];
/// Signals a search considers (newest first).
const SCAN_LIMIT: i64 = 5000;

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignalUser {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignalQuery {
    pub name: String,
    pub query: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SecuritySignal {
    pub id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub rule_type: String,
    pub title: String,
    pub message: String,
    /// info | low | medium | high | critical
    pub severity: String,
    pub case_name: String,
    pub condition: String,
    pub group_values: BTreeMap<String, String>,
    pub queries: Vec<SignalQuery>,
    /// The logs the signal is about, in the logs search syntax (the first query and the group).
    pub logs_query: String,
    pub source: String,
    pub services: Vec<String>,
    pub env: String,
    pub tags: Vec<String>,
    /// `{ip: {top: [...], count: n}, user: …, host: …, service: …, country: …}`
    #[schema(value_type = Object)]
    pub entities: Value,
    pub events_matched: i64,
    /// Epoch ms.
    pub first_seen: i64,
    pub last_seen: i64,
    pub created_at: i64,
    /// open | under_review | archived
    pub state: String,
    pub archive_reason: String,
    pub archive_comment: String,
    pub assignee: Option<SignalUser>,
    pub state_updated_at: Option<i64>,
    pub state_updated_by: Option<String>,
    pub incident_ids: Vec<i64>,
}

#[derive(sqlx::FromRow)]
struct SignalRow {
    id: String,
    rule_id: String,
    rule_name: String,
    rule_type: String,
    title: String,
    message: String,
    severity: String,
    case_name: String,
    condition: String,
    group_values: Value,
    queries: Value,
    source: String,
    services: Vec<String>,
    env: String,
    tags: Vec<String>,
    entities: Value,
    events_matched: i64,
    first_seen: i64,
    last_seen: i64,
    created_at: i64,
    state: String,
    archive_reason: String,
    archive_comment: String,
    assignee_id: Option<String>,
    assignee_name: Option<String>,
    assignee_email: Option<String>,
    state_updated_at: Option<i64>,
    state_updated_by: Option<String>,
    incident_ids: Vec<i64>,
}

const SELECT: &str = "SELECT s.id, s.rule_id, s.rule_name, s.rule_type, s.title, s.message, s.severity, s.case_name, s.condition, s.group_values, s.queries, s.source, \
    s.services, s.env, s.tags, s.entities, s.events_matched, (extract(epoch FROM s.first_seen) * 1000)::bigint AS first_seen, \
    (extract(epoch FROM s.last_seen) * 1000)::bigint AS last_seen, (extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, s.state, s.archive_reason, \
    s.archive_comment, s.assignee_id::text AS assignee_id, a.name AS assignee_name, a.email AS assignee_email, \
    (extract(epoch FROM s.state_updated_at) * 1000)::bigint AS state_updated_at, u.name AS state_updated_by, s.incident_ids \
    FROM security_signals s LEFT JOIN users a ON a.id = s.assignee_id AND a.org_id = s.org_id LEFT JOIN users u ON u.id = s.state_updated_by AND u.org_id = s.org_id";

/// `key:"value"` with quotes when the value needs them.
fn term(key: &str, value: &str) -> String {
    if value.chars().any(|c| c.is_whitespace() || "():\"".contains(c)) {
        format!("{key}:\"{}\"", value.replace('"', "\\\""))
    } else {
        format!("{key}:{value}")
    }
}

impl From<SignalRow> for SecuritySignal {
    fn from(r: SignalRow) -> Self {
        let group_values: BTreeMap<String, String> = serde_json::from_value(r.group_values).unwrap_or_default();
        let queries: Vec<SignalQuery> = r
            .queries
            .as_array()
            .map(|a| a.iter().map(|q| SignalQuery { name: q["name"].as_str().unwrap_or_default().into(), query: q["query"].as_str().unwrap_or_default().into() }).collect())
            .unwrap_or_default();
        let mut logs_query = queries.first().map(|q| q.query.clone()).unwrap_or_default();
        for (k, v) in &group_values {
            logs_query.push(' ');
            logs_query.push_str(&term(k, v));
        }
        SecuritySignal {
            assignee: r.assignee_id.map(|id| SignalUser { id, name: r.assignee_name.unwrap_or_default(), email: r.assignee_email.unwrap_or_default() }),
            id: r.id,
            rule_id: r.rule_id,
            rule_name: r.rule_name,
            rule_type: r.rule_type,
            title: r.title,
            message: r.message,
            severity: r.severity,
            case_name: r.case_name,
            condition: r.condition,
            group_values,
            queries,
            logs_query: logs_query.trim().to_string(),
            source: r.source,
            services: r.services,
            env: r.env,
            tags: r.tags,
            entities: r.entities,
            events_matched: r.events_matched,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
            created_at: r.created_at,
            state: r.state,
            archive_reason: r.archive_reason,
            archive_comment: r.archive_comment,
            state_updated_at: r.state_updated_at,
            state_updated_by: r.state_updated_by,
            incident_ids: r.incident_ids,
        }
    }
}

/// The values a signal has for a search key.
fn values_of(s: &SecuritySignal, key: &str) -> Vec<String> {
    let k = key.trim_start_matches('@');
    let one = |v: &str| if v.is_empty() { vec![] } else { vec![v.to_string()] };
    match k {
        "status" | "severity" => vec![s.severity.clone()],
        "state" | "workflow.triage.state" => vec![s.state.clone()],
        "archive_reason" | "workflow.triage.archive_reason" => vec![s.archive_reason.clone()],
        "source" => one(&s.source),
        "service" => s.services.clone(),
        "env" => one(&s.env),
        "rule" | "ruleId" | "rule_id" | "workflow.rule.id" => vec![s.rule_id.clone()],
        "rule_name" | "workflow.rule.name" => vec![s.rule_name.clone()],
        "type" | "workflow.rule.type" => vec![s.rule_type.clone()],
        "assignee" | "workflow.triage.assignee.name" => s.assignee.as_ref().map(|a| vec![a.name.clone(), a.email.clone()]).unwrap_or_else(|| vec!["unassigned".into()]),
        "has_case" | "workflow.triage.has_case" => vec![(!s.incident_ids.is_empty()).to_string()],
        _ => {
            let mut out: Vec<String> = s.tags.iter().filter_map(|t| t.split_once(':')).filter(|(tk, _)| *tk == k).map(|(_, v)| v.to_string()).collect();
            if let Some(v) = s.group_values.get(key).or_else(|| s.group_values.get(&format!("@{k}"))).or_else(|| s.group_values.get(k)) {
                out.push(v.clone());
            }
            out
        }
    }
}

fn glob(pattern: &str, value: &str) -> bool {
    let (p, v) = (pattern.to_lowercase(), value.to_lowercase());
    match (p.strip_prefix('*'), p.strip_suffix('*')) {
        _ if !p.contains('*') => p == v,
        (Some(rest), _) if rest.ends_with('*') => v.contains(rest.trim_end_matches('*')),
        (Some(rest), _) => v.ends_with(rest),
        (None, Some(rest)) => v.starts_with(rest),
        _ => {
            let (a, b) = p.split_once('*').unwrap();
            v.starts_with(a) && v.ends_with(b)
        }
    }
}

pub fn matches(s: &SecuritySignal, terms: &[Term]) -> bool {
    terms.iter().all(|t| {
        let (hit, negate) = match t {
            Term::Tag { key, values, negate } => {
                let got = values_of(s, key);
                (values.iter().any(|p| got.iter().any(|v| glob(p, v))), *negate)
            }
            Term::Text { text, negate } => {
                let t = text.to_lowercase();
                (s.title.to_lowercase().contains(&t) || s.rule_name.to_lowercase().contains(&t), *negate)
            }
        };
        hit != negate
    })
}

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignalSearchRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// open | under_review | archived; empty: all.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    /// Oldest first (default newest first).
    #[serde(default)]
    pub ascending: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalFacet {
    /// The search key (`status`, `@workflow.triage.state`, `source`, …).
    pub key: String,
    pub values: Vec<SignalFacetValue>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalBucket {
    pub ts: i64,
    pub info: usize,
    pub low: usize,
    pub medium: usize,
    pub high: usize,
    pub critical: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalSearchPage {
    pub total: usize,
    /// Counts per triage state of the signals matching the query (the All / Open / Archived / Under Review tabs).
    pub states: BTreeMap<String, usize>,
    pub signals: Vec<SecuritySignal>,
    pub facets: Vec<SignalFacet>,
    pub histogram: Vec<SignalBucket>,
}

const FACETS: [&str; 9] = ["@workflow.triage.state", "status", "source", "@workflow.rule.type", "@workflow.rule.name", "service", "env", "assignee", "tactic"];

pub async fn signals_in(st: &AppState, org: &str, range: TimeRange) -> Result<Vec<SecuritySignal>, ApiError> {
    let rows: Vec<SignalRow> = sqlx::query_as(&format!(
        "{SELECT} WHERE s.org_id = $1 AND s.last_seen >= to_timestamp($2 / 1000.0) AND s.created_at <= to_timestamp($3 / 1000.0) ORDER BY s.created_at DESC LIMIT {SCAN_LIMIT}"
    ))
    .bind(org)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().map(SecuritySignal::from).collect())
}

/// One facet's value counts.
fn facet(key: &str, signals: &[&SecuritySignal]) -> SignalFacet {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in signals {
        let vals = match key {
            "@workflow.rule.name" => vec![s.rule_name.clone()],
            "assignee" => vec![s.assignee.as_ref().map_or("Unassigned".to_string(), |a| a.name.clone())],
            _ => values_of(s, key),
        };
        for v in vals {
            *counts.entry(v).or_default() += 1;
        }
    }
    let mut values: Vec<SignalFacetValue> = counts.into_iter().map(|(value, count)| SignalFacetValue { value, count }).collect();
    values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
    values.truncate(25);
    SignalFacet { key: key.to_string(), values }
}

fn histogram(range: TimeRange, signals: &[&SecuritySignal]) -> Vec<SignalBucket> {
    let span = (range.to - range.from).max(60_000);
    let step = [60_000, 300_000, 900_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 86_400_000].into_iter().find(|s| span / s <= 120).unwrap_or(86_400_000);
    let start = range.from - range.from.rem_euclid(step);
    let mut buckets: Vec<SignalBucket> = (0..).map(|i| start + i * step).take_while(|t| *t <= range.to).map(|ts| SignalBucket { ts, info: 0, low: 0, medium: 0, high: 0, critical: 0 }).collect();
    for s in signals {
        let i = ((s.created_at - start) / step) as usize;
        if let Some(b) = buckets.get_mut(i) {
            match s.severity.as_str() {
                "critical" => b.critical += 1,
                "high" => b.high += 1,
                "medium" => b.medium += 1,
                "low" => b.low += 1,
                _ => b.info += 1,
            }
        }
    }
    buckets
}

#[utoipa::path(post, path = "/api/v1/security/signals/search", operation_id = "search_security_signals", request_body = SignalSearchRequest,
    responses((status = 200, body = SignalSearchPage)), tag = "security")]
pub async fn search(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SignalSearchRequest>) -> ApiResult<SignalSearchPage> {
    let all = signals_in(&st, &ctx.org_id, req.range).await?;
    let terms = parse_query(&req.query);
    let matched: Vec<&SecuritySignal> = all.iter().filter(|s| s.created_at >= req.range.from && matches(s, &terms)).collect();
    let mut states: BTreeMap<String, usize> = STATES.iter().map(|s| (s.to_string(), 0)).collect();
    for s in &matched {
        *states.entry(s.state.clone()).or_default() += 1;
    }
    let mut shown: Vec<&SecuritySignal> = matched.iter().copied().filter(|s| req.state.is_empty() || s.state == req.state).collect();
    if req.ascending {
        shown.reverse();
    }
    let facets = FACETS.iter().map(|k| facet(k, &shown)).collect();
    let histogram = histogram(req.range, &shown);
    let total = shown.len();
    let offset = req.offset.unwrap_or(0);
    let limit = req.limit.unwrap_or(100).min(500);
    Ok(Json(SignalSearchPage { total, states, signals: shown.into_iter().skip(offset).take(limit).cloned().collect(), facets, histogram }))
}

pub async fn load_signal(st: &AppState, org: &str, id: &str) -> Result<SecuritySignal, ApiError> {
    let row: Option<SignalRow> = sqlx::query_as(&format!("{SELECT} WHERE s.org_id = $1 AND s.id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(SecuritySignal::from).ok_or_else(|| ApiError::NotFound(format!("no signal {id}")))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalDetail {
    #[serde(flatten)]
    pub signal: SecuritySignal,
    /// Signals of the same rule in the last 30 days and the last day.
    pub rule_signals_month: i64,
    pub rule_signals_day: i64,
    /// Whether the rule still exists (and its id, to open it).
    pub rule_exists: bool,
}

#[utoipa::path(get, path = "/api/v1/security/signals/{id}", operation_id = "get_security_signal", params(("id" = String, Path)),
    responses((status = 200, body = SignalDetail), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn get(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<SignalDetail> {
    let signal = load_signal(&st, &ctx.org_id, &id).await?;
    let (month, day, exists): (i64, i64, bool) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE created_at > now() - interval '30 days'), count(*) FILTER (WHERE created_at > now() - interval '1 day'), \
         EXISTS (SELECT 1 FROM security_rules WHERE org_id = $1 AND id = $2) FROM security_signals WHERE org_id = $1 AND rule_id = $2",
    )
    .bind(&ctx.org_id)
    .bind(&signal.rule_id)
    .fetch_one(&st.postgres)
    .await?;
    Ok(Json(SignalDetail { signal, rule_signals_month: month, rule_signals_day: day, rule_exists: exists }))
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SignalStateInput {
    /// open | under_review | archived
    pub state: String,
    #[serde(default)]
    pub archive_reason: Option<String>,
    #[serde(default)]
    pub archive_comment: Option<String>,
}

pub fn check_state(input: &SignalStateInput) -> Result<(), String> {
    if !STATES.contains(&input.state.as_str()) {
        return Err(format!("invalid state `{}`: use open, under_review or archived", input.state));
    }
    if let Some(r) = &input.archive_reason {
        if !ARCHIVE_REASONS.contains(&r.as_str()) {
            return Err(format!("invalid archive reason `{r}`"));
        }
    }
    Ok(())
}

/// Sets the triage state of signals (unknown ids are skipped); returns how many changed.
pub async fn set_state(st: &AppState, ctx: &AuthContext, ids: &[String], input: &SignalStateInput) -> Result<u64, ApiError> {
    check_state(input).map_err(ApiError::BadRequest)?;
    let reason = if input.state == "archived" { input.archive_reason.clone().unwrap_or_else(|| "other".into()) } else { "none".into() };
    Ok(sqlx::query(
        "UPDATE security_signals SET state = $3, archive_reason = $4, archive_comment = $5, state_updated_at = now(), state_updated_by = $6::uuid \
         WHERE org_id = $1 AND id = ANY($2)",
    )
    .bind(&ctx.org_id)
    .bind(ids)
    .bind(&input.state)
    .bind(reason)
    .bind(input.archive_comment.clone().unwrap_or_default())
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?
    .rows_affected())
}

/// Assigns signals to a member of the org (`None`: unassigned).
pub async fn set_assignee(st: &AppState, ctx: &AuthContext, ids: &[String], assignee: Option<&str>) -> Result<u64, ApiError> {
    if let Some(a) = assignee {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE org_id = $1 AND id::text = $2 AND status = 'active')").bind(&ctx.org_id).bind(a).fetch_one(&st.postgres).await?;
        if !ok {
            return Err(ApiError::BadRequest(format!("{a} is not an active member of the organization")));
        }
    }
    Ok(sqlx::query("UPDATE security_signals SET assignee_id = $3::uuid WHERE org_id = $1 AND id = ANY($2)").bind(&ctx.org_id).bind(ids).bind(assignee).execute(&st.postgres).await?.rows_affected())
}

#[utoipa::path(put, path = "/api/v1/security/signals/{id}/state", operation_id = "set_security_signal_state", params(("id" = String, Path)), request_body = SignalStateInput,
    responses((status = 200, body = SignalDetail), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn state(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SignalStateInput>) -> ApiResult<SignalDetail> {
    load_signal(&st, &ctx.org_id, &id).await?;
    set_state(&st, &ctx, std::slice::from_ref(&id), &input).await?;
    get(State(st), ctx, Path(id)).await
}

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignalAssigneeInput {
    /// A member's id; null unassigns.
    pub assignee_id: Option<String>,
}

#[utoipa::path(put, path = "/api/v1/security/signals/{id}/assignee", operation_id = "set_security_signal_assignee", params(("id" = String, Path)), request_body = SignalAssigneeInput,
    responses((status = 200, body = SignalDetail), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn assignee(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SignalAssigneeInput>) -> ApiResult<SignalDetail> {
    load_signal(&st, &ctx.org_id, &id).await?;
    set_assignee(&st, &ctx, std::slice::from_ref(&id), input.assignee_id.as_deref()).await?;
    get(State(st), ctx, Path(id)).await
}

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignalBulkInput {
    pub ids: Vec<String>,
    pub state: Option<SignalStateInput>,
    /// Set to change the assignee; `""` unassigns.
    pub assignee_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct SignalBulkResult {
    pub updated: u64,
}

#[utoipa::path(post, path = "/api/v1/security/signals/bulk", operation_id = "bulk_update_security_signals", request_body = SignalBulkInput,
    responses((status = 200, body = SignalBulkResult), (status = 400, body = crate::error::ErrorBody)), tag = "security")]
pub async fn bulk(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<SignalBulkInput>) -> ApiResult<SignalBulkResult> {
    if input.ids.is_empty() || input.ids.len() > 1000 {
        return Err(ApiError::BadRequest("select between 1 and 1000 signals".into()));
    }
    let mut updated = 0;
    if let Some(s) = &input.state {
        updated = updated.max(set_state(&st, &ctx, &input.ids, s).await?);
    }
    if let Some(a) = &input.assignee_id {
        updated = updated.max(set_assignee(&st, &ctx, &input.ids, if a.is_empty() { None } else { Some(a) }).await?);
    }
    Ok(Json(SignalBulkResult { updated }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn signal() -> SecuritySignal {
        SecuritySignal {
            id: "AQAAA1".into(),
            rule_id: "def-web-001".into(),
            rule_name: "Attack attempts on a web service".into(),
            rule_type: "log_detection".into(),
            title: "Attack attempts on a web service - storefront-api - Condition 1".into(),
            message: String::new(),
            severity: "info".into(),
            case_name: "Condition 1".into(),
            condition: "a > 100".into(),
            group_values: [("service".to_string(), "storefront-api".to_string())].into(),
            queries: vec![SignalQuery { name: "a".into(), query: "source:nginx @security.attack:true".into() }],
            logs_query: String::new(),
            source: "nginx".into(),
            services: vec!["storefront-api".into()],
            env: "prod".into(),
            tags: vec!["tactic:TA0043-reconnaissance".into(), "source:nginx".into()],
            entities: json!({}),
            events_matched: 2160,
            first_seen: 0,
            last_seen: 0,
            created_at: 0,
            state: "open".into(),
            archive_reason: "none".into(),
            archive_comment: String::new(),
            assignee: None,
            state_updated_at: None,
            state_updated_by: None,
            incident_ids: vec![],
        }
    }

    #[test]
    fn search_terms_match_signal_fields() {
        let s = signal();
        for q in ["status:info", "source:nginx", "service:storefront-api", "@workflow.triage.state:open", "tactic:TA0043*", "attack", "-status:high", "assignee:unassigned", "status:(info OR low)"] {
            assert!(matches(&s, &parse_query(q)), "{q}");
        }
        for q in ["status:high", "source:okta", "-env:prod", "checkout"] {
            assert!(!matches(&s, &parse_query(q)), "{q}");
        }
    }

    #[test]
    fn states_are_checked() {
        assert!(check_state(&SignalStateInput { state: "archived".into(), archive_reason: Some("false_positive".into()), archive_comment: None }).is_ok());
        assert!(check_state(&SignalStateInput { state: "closed".into(), archive_reason: None, archive_comment: None }).is_err());
        assert!(check_state(&SignalStateInput { state: "archived".into(), archive_reason: Some("bored".into()), archive_comment: None }).is_err());
    }

    #[test]
    fn logs_query_adds_the_group() {
        assert_eq!(term("@usr.email", "ana@example.com"), "@usr.email:ana@example.com");
        assert_eq!(term("@usr.name", "system:anonymous"), "@usr.name:\"system:anonymous\"");
    }
}
