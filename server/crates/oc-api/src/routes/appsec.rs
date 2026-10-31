//! App & API Protection (Security → App & API Protection): what the tracers' In-App WAF reports on the
//! spans, read from `otel.otel_traces`.
//!
//! - **Security traces** are entry spans with WAF triggers (`_dd.appsec.json`, as Datadog's tracers
//!   write it: rules with their `type` and `category` tags and what matched) or business logic events
//!   (`appsec.events.users.login.{success,failure}.track`). `appsec.blocked` marks blocked requests,
//!   `_dd.appsec.enabled` the services running App & API Protection. Client IP, method, route, status
//!   and user agent are read under Datadog's and OpenTelemetry's attribute names.
//! - **Endpoints** are the entry spans' (service, method, route); **API findings** are our own
//!   checks of them ([`FINDING_RULES`], mapped to the OWASP API Security Top 10 2023).
//! - **Attackers** are client IPs with security traces, **users** the `usr.id` of the traces.
//! - The denylist and settings live in Postgres; the WAF configuration is
//!   [`super::datadog_api::appsec`]'s. Blocking needs Remote Configuration, which ObserveCraft doesn't
//!   serve yet: the configuration is kept and shown.
//! - Detection rules of type `application_security` are evaluated by [`super::security_engine`] with
//!   [`group_counts`] and [`entities`] over the security traces.
//!
//! Tenancy: ClickHouse reads filter on `OrgId = {oc_org:String}` through `AppState::ch(org)`, Postgres
//! ones on `org_id`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{build_where, parse_query, Param, Schema, SpansSchema, Term};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::datadog_api::appsec::{self as waf, Kind, MANAGED_POLICIES};
use super::logs::{bind, TimeRange};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const DAY: i64 = 86_400_000;

const ENTRY: &str = "SpanKind IN ('Server', 'Consumer')";
const IP: &str = "if(SpanAttributes['http.client_ip'] != '', SpanAttributes['http.client_ip'], SpanAttributes['network.client.ip'])";
const METHOD: &str = "if(SpanAttributes['http.method'] != '', SpanAttributes['http.method'], SpanAttributes['http.request.method'])";
const ROUTE: &str = "if(SpanAttributes['http.route'] != '', SpanAttributes['http.route'], SpanName)";
const STATUS: &str = "if(SpanAttributes['http.status_code'] != '', SpanAttributes['http.status_code'], SpanAttributes['http.response.status_code'])";
const UA: &str = "if(SpanAttributes['http.useragent'] != '', SpanAttributes['http.useragent'], SpanAttributes['user_agent.original'])";
const URL: &str = "if(SpanAttributes['http.url'] != '', SpanAttributes['http.url'], SpanAttributes['url.full'])";
const COUNTRY: &str = "SpanAttributes['network.client.geoip.country.iso_code']";
const ENV: &str = "ResourceAttributes['deployment.environment']";
const TYPES: &str = "arrayDistinct(arrayMap(x -> JSONExtractString(x, 'rule', 'tags', 'type'), JSONExtractArrayRaw(SpanAttributes['_dd.appsec.json'], 'triggers')))";
const CATEGORIES: &str = "arrayDistinct(arrayMap(x -> JSONExtractString(x, 'rule', 'tags', 'category'), JSONExtractArrayRaw(SpanAttributes['_dd.appsec.json'], 'triggers')))";
const ATTACK: &str = "SpanAttributes['_dd.appsec.json'] != ''";
const LOGIN_OK: &str = "SpanAttributes['appsec.events.users.login.success.track'] = 'true'";
const LOGIN_FAIL: &str = "SpanAttributes['appsec.events.users.login.failure.track'] = 'true'";
const BLOCKED: &str = "SpanAttributes['appsec.blocked'] = 'true'";
const ENABLED: &str = "SpanAttributes['_dd.appsec.enabled'] IN ('1', '1.0', 'true')";

fn security() -> String {
    format!("({ATTACK} OR {LOGIN_OK} OR {LOGIN_FAIL})")
}

/// The SQL of a search key: the derived security fields, then the spans' own.
fn field(key: &str) -> (String, Option<Param>) {
    match key.trim_start_matches('@') {
        "http.client_ip" | "network.client.ip" | "ip" => (IP.into(), None),
        "http.method" => (METHOD.into(), None),
        "http.route" | "route" => (ROUTE.into(), None),
        "http.status_code" => (STATUS.into(), None),
        "http.useragent" => (UA.into(), None),
        "country" | "network.client.geoip.country.iso_code" => (COUNTRY.into(), None),
        "appsec.blocked" => ("toString(SpanAttributes['appsec.blocked'] = 'true')".into(), None),
        "usr.id" => ("SpanAttributes['usr.id']".into(), None),
        _ => SpansSchema.field(key),
    }
}

/// A search over the security traces: `@appsec.type:` / `@appsec.category:` / `@appsec.rule_id:` match
/// any of the trace's triggers, the derived fields as in [`field`], the rest as span searches.
pub fn search_clause(query: &str) -> (String, Vec<Param>) {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    let mut rest = Vec::new();
    for t in parse_query(query) {
        match &t {
            Term::Tag { key, values, negate } => {
                let k = key.trim_start_matches('@');
                let array = match k {
                    "appsec.type" => Some(TYPES.to_string()),
                    "appsec.category" => Some(CATEGORIES.to_string()),
                    "appsec.rule_id" => Some("arrayMap(x -> JSONExtractString(x, 'rule', 'id'), JSONExtractArrayRaw(SpanAttributes['_dd.appsec.json'], 'triggers'))".to_string()),
                    _ => None,
                };
                let (expr, keyp) = match &array {
                    Some(_) => (String::new(), None),
                    None if matches!(k, "http.client_ip" | "network.client.ip" | "ip" | "http.method" | "http.route" | "route" | "http.status_code" | "http.useragent" | "country" | "appsec.blocked" | "usr.id") => field(key),
                    None => {
                        rest.push(t.clone());
                        continue;
                    }
                };
                let mut ors = Vec::new();
                for v in values {
                    let like = v.contains('*');
                    let pat = v.replace('*', "%");
                    match &array {
                        Some(a) if like => ors.push(format!("arrayExists(x -> x LIKE ?, {a})")),
                        Some(a) => ors.push(format!("has({a}, ?)")),
                        None => {
                            params.extend(keyp.clone());
                            ors.push(format!("{expr} {} ?", if like { "LIKE" } else { "=" }));
                        }
                    }
                    params.push(Param::Str(if like { pat } else { v.clone() }));
                }
                let c = format!("({})", ors.join(" OR "));
                clauses.push(if *negate { format!("NOT {c}") } else { c });
            }
            Term::Text { .. } => rest.push(t.clone()),
        }
    }
    let f = build_where(&SpansSchema, &rest, None);
    let mut out = vec![f.clause];
    let mut all = f.params;
    out.extend(clauses);
    all.extend(params);
    (out.join(" AND "), all)
}

fn range_clause(r: TimeRange) -> (String, Vec<Param>) {
    ("Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".into(), vec![Param::Int(r.from), Param::Int(r.to)])
}

// ================================================================ security traces

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WafTrigger {
    pub rule_id: String,
    pub rule_name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub category: String,
    /// What matched: `address` (server.request.query, …), key path and value.
    pub matches: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SecurityTrace {
    pub trace_id: String,
    pub span_id: String,
    /// Epoch ms.
    pub timestamp: i64,
    pub service: String,
    pub env: String,
    pub method: String,
    pub route: String,
    pub url: String,
    pub status: String,
    pub blocked: bool,
    pub client_ip: String,
    pub country: String,
    pub user_agent: String,
    pub user: String,
    /// attack_attempt.sql_injection, business_logic.users.login.failure, security_response.block_ip, …
    pub activities: Vec<String>,
    pub triggers: Vec<WafTrigger>,
}

#[derive(clickhouse::Row, Deserialize)]
struct TraceRow {
    trace_id: String,
    span_id: String,
    ts: i64,
    service: String,
    env: String,
    method: String,
    route: String,
    url: String,
    status: String,
    blocked: u8,
    ip: String,
    country: String,
    ua: String,
    user: String,
    json: String,
    login_ok: u8,
    login_fail: u8,
}

/// The triggers of a `_dd.appsec.json`.
pub fn triggers(json: &str) -> Vec<WafTrigger> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    v.get("triggers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| {
            let s = |p: &str| t.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string();
            let matches = t
                .get("rule_matches")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|m| m.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default())
                .map(|p| {
                    let addr = p.get("address").and_then(Value::as_str).unwrap_or_default();
                    let path = p.get("key_path").and_then(Value::as_array).map(|k| k.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(".")).unwrap_or_default();
                    let value = p.get("value").and_then(Value::as_str).unwrap_or_default();
                    format!("{addr}{}{}: {value}", if path.is_empty() { "" } else { "." }, path)
                })
                .take(5)
                .collect();
            WafTrigger { rule_id: s("/rule/id"), rule_name: s("/rule/name"), kind: s("/rule/tags/type"), category: s("/rule/tags/category"), matches }
        })
        .collect()
}

fn to_trace(r: TraceRow) -> SecurityTrace {
    let triggers = triggers(&r.json);
    let mut activities: Vec<String> = triggers.iter().map(|t| format!("{}.{}", if t.category.is_empty() { "attack_attempt" } else { &t.category }, t.kind)).collect::<BTreeSet<_>>().into_iter().collect();
    if r.login_ok == 1 {
        activities.push("business_logic.users.login.success".into());
    }
    if r.login_fail == 1 {
        activities.push("business_logic.users.login.failure".into());
    }
    if r.blocked == 1 {
        activities.push("security_response.block_ip".into());
    }
    SecurityTrace {
        trace_id: r.trace_id,
        span_id: r.span_id,
        timestamp: r.ts,
        service: r.service,
        env: r.env,
        method: r.method,
        route: r.route,
        url: r.url,
        status: r.status,
        blocked: r.blocked == 1,
        client_ip: r.ip,
        country: r.country,
        user_agent: r.ua,
        user: r.user,
        activities,
        triggers,
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppsecSearchRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppsecFacet {
    pub key: String,
    pub values: Vec<NamedCount>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeBucket {
    pub ts: i64,
    /// Count per security activity type.
    pub counts: BTreeMap<String, u64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityTracePage {
    pub total: u64,
    pub traces: Vec<SecurityTrace>,
    pub facets: Vec<AppsecFacet>,
    pub histogram: Vec<TypeBucket>,
    pub top_activities: Vec<NamedCount>,
    pub top_users: Vec<NamedCount>,
}

#[derive(clickhouse::Row, Deserialize)]
struct KV {
    k: String,
    n: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Tkv {
    t: i64,
    k: String,
    n: u64,
}

/// The common WHERE (after the org filter each query writes itself): range, entry spans, security traces, and the search.
fn traces_where(range: TimeRange, query: &str) -> (String, Vec<Param>) {
    let (rc, mut params) = range_clause(range);
    let (sc, sp) = search_clause(query);
    params.extend(sp);
    (format!("{rc} AND {ENTRY} AND {} AND ({sc})", security()), params)
}

async fn kv(st: &AppState, org: &str, sql: String, params: &[Param]) -> Result<Vec<KV>, ApiError> {
    Ok(bind(st.ch(org).query(&sql), params).fetch_all::<KV>().await?)
}

/// An activity's label from the trace: its WAF types, login events or blocking.
const ACTIVITY: &str = "arrayConcat(arrayMap(x -> concat(if(JSONExtractString(x, 'rule', 'tags', 'category') = '', 'attack_attempt', JSONExtractString(x, 'rule', 'tags', 'category')), '.', JSONExtractString(x, 'rule', 'tags', 'type')), JSONExtractArrayRaw(SpanAttributes['_dd.appsec.json'], 'triggers')), \
    if(SpanAttributes['appsec.events.users.login.success.track'] = 'true', ['business_logic.users.login.success'], []), if(SpanAttributes['appsec.events.users.login.failure.track'] = 'true', ['business_logic.users.login.failure'], []), \
    if(SpanAttributes['appsec.blocked'] = 'true', ['security_response.block_ip'], []))";

pub async fn search_traces(st: &AppState, org: &str, req: &AppsecSearchRequest) -> Result<SecurityTracePage, ApiError> {
    let (w, p) = traces_where(req.range, &req.query);
    let limit = req.limit.unwrap_or(200).min(1000);
    let rows: Vec<TraceRow> = bind(
        st.ch(org).query(&format!(
            "SELECT TraceId AS trace_id, SpanId AS span_id, toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, ServiceName AS service, {ENV} AS env, {METHOD} AS method, {ROUTE} AS route, \
             {URL} AS url, {STATUS} AS status, toUInt8({BLOCKED}) AS blocked, {IP} AS ip, {COUNTRY} AS country, {UA} AS ua, SpanAttributes['usr.id'] AS user, \
             SpanAttributes['_dd.appsec.json'] AS json, toUInt8({LOGIN_OK}) AS login_ok, toUInt8({LOGIN_FAIL}) AS login_fail \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} ORDER BY Timestamp DESC LIMIT {limit}"
        )),
        &p,
    )
    .fetch_all()
    .await?;
    let total: u64 = bind(st.ch(org).query(&format!("SELECT count() FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w}")), &p).fetch_one().await?;
    let mut facets = Vec::new();
    for (key, expr) in [
        ("service", "ServiceName".to_string()),
        ("env", ENV.to_string()),
        ("@appsec.type", format!("arrayJoin(if(empty({TYPES}), [''], {TYPES}))")),
        ("@appsec.category", format!("arrayJoin(if(empty({CATEGORIES}), [''], {CATEGORIES}))")),
        ("@http.status_code", STATUS.to_string()),
        ("@appsec.blocked", format!("toString({BLOCKED})")),
        ("@http.client_ip", IP.to_string()),
        ("country", COUNTRY.to_string()),
        ("@http.route", ROUTE.to_string()),
    ] {
        let values = kv(st, org, format!("SELECT toString({expr}) AS k, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY k HAVING k != '' ORDER BY n DESC LIMIT 25"), &p).await?;
        facets.push(AppsecFacet { key: key.into(), values: values.into_iter().map(|r| NamedCount { name: r.k, count: r.n }).collect() });
    }
    let span = (req.range.to - req.range.from).max(60_000);
    let step = [60_000, 300_000, 900_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, DAY].into_iter().find(|s| span / s <= 120).unwrap_or(DAY);
    let buckets: Vec<Tkv> = bind(
        st.ch(org).query(&format!(
            "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, arrayJoin({ACTIVITY}) AS k, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY t, k ORDER BY t"
        )),
        &p,
    )
    .fetch_all()
    .await?;
    let mut hist: BTreeMap<i64, BTreeMap<String, u64>> = BTreeMap::new();
    let mut acts: HashMap<String, u64> = HashMap::new();
    for b in buckets {
        *acts.entry(b.k.clone()).or_default() += b.n;
        hist.entry(b.t).or_default().insert(b.k, b.n);
    }
    let mut top_activities: Vec<NamedCount> = acts.into_iter().map(|(name, count)| NamedCount { name, count }).collect();
    top_activities.sort_by_key(|b| std::cmp::Reverse(b.count));
    top_activities.truncate(8);
    let users = kv(st, org, format!("SELECT SpanAttributes['usr.id'] AS k, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND k != '' GROUP BY k ORDER BY n DESC LIMIT 5"), &p).await?;
    Ok(SecurityTracePage {
        total,
        traces: rows.into_iter().map(to_trace).collect(),
        facets,
        histogram: hist.into_iter().map(|(ts, counts)| TypeBucket { ts, counts }).collect(),
        top_activities,
        top_users: users.into_iter().map(|r| NamedCount { name: r.k, count: r.n }).collect(),
    })
}

#[utoipa::path(post, path = "/api/v1/appsec/traces", operation_id = "appsec_security_traces", request_body = AppsecSearchRequest, responses((status = 200, body = SecurityTracePage)), tag = "appsec")]
pub async fn traces(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AppsecSearchRequest>) -> ApiResult<SecurityTracePage> {
    Ok(Json(search_traces(&st, &ctx.org_id, &req).await?))
}

// ================================================================ attackers and users

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Attacker {
    pub ip: String,
    pub country: String,
    pub services: Vec<String>,
    pub envs: Vec<String>,
    pub user_agents: Vec<String>,
    pub attack_types: Vec<String>,
    pub traces: u64,
    pub attacks: u64,
    pub blocked_traces: u64,
    /// Epoch ms.
    pub first_seen: i64,
    pub last_seen: i64,
    /// On the denylist.
    pub denylisted: bool,
    /// Many attacks from several tools (scanner, injections): worth blocking.
    pub suspicious: bool,
}

#[derive(clickhouse::Row, Deserialize)]
struct AttackerRow {
    ip: String,
    country: String,
    services: Vec<String>,
    envs: Vec<String>,
    uas: Vec<String>,
    types: Vec<String>,
    traces: u64,
    attacks: u64,
    blocked: u64,
    first: i64,
    last: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttackerPage {
    pub attackers: Vec<Attacker>,
    pub suspicious: usize,
    pub flagged: usize,
}

pub async fn attackers_of(st: &AppState, org: &str, range: TimeRange, query: &str) -> Result<Vec<Attacker>, ApiError> {
    let (w, p) = traces_where(range, query);
    let rows: Vec<AttackerRow> = bind(
        st.ch(org).query(&format!(
            "SELECT {IP} AS ip, any({COUNTRY}) AS country, groupUniqArray(10)(ServiceName) AS services, groupUniqArray(5)({ENV}) AS envs, groupUniqArray(5)({UA}) AS uas, \
             groupUniqArrayArray(10)({TYPES}) AS types, count() AS traces, countIf({ATTACK}) AS attacks, countIf({BLOCKED}) AS blocked, \
             toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND ip != '' GROUP BY ip ORDER BY attacks DESC, traces DESC LIMIT 500"
        )),
        &p,
    )
    .fetch_all()
    .await?;
    let deny: BTreeSet<String> = sqlx::query_scalar("SELECT value FROM appsec_denylist WHERE org_id = $1 AND kind = 'ip' AND (expires_at IS NULL OR expires_at > now())").bind(org).fetch_all(&st.postgres).await?.into_iter().collect();
    Ok(rows
        .into_iter()
        .map(|r| Attacker {
            denylisted: deny.contains(&r.ip),
            suspicious: r.attacks >= 20 && r.types.iter().filter(|t| !t.is_empty()).count() >= 2,
            ip: r.ip,
            country: r.country,
            services: r.services,
            envs: r.envs.into_iter().filter(|e| !e.is_empty()).collect(),
            user_agents: r.uas.into_iter().filter(|u| !u.is_empty()).collect(),
            attack_types: r.types.into_iter().filter(|t| !t.is_empty()).collect(),
            traces: r.traces,
            attacks: r.attacks,
            blocked_traces: r.blocked,
            first_seen: r.first,
            last_seen: r.last,
        })
        .collect())
}

#[utoipa::path(post, path = "/api/v1/appsec/attackers", operation_id = "appsec_attackers", request_body = AppsecSearchRequest, responses((status = 200, body = AttackerPage)), tag = "appsec")]
pub async fn attackers(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AppsecSearchRequest>) -> ApiResult<AttackerPage> {
    let attackers = attackers_of(&st, &ctx.org_id, req.range, &req.query).await?;
    Ok(Json(AttackerPage { suspicious: attackers.iter().filter(|a| a.suspicious && !a.denylisted).count(), flagged: attackers.iter().filter(|a| a.denylisted).count(), attackers }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppsecUser {
    pub user: String,
    pub ips: Vec<String>,
    pub countries: Vec<String>,
    pub services: Vec<String>,
    pub login_success: u64,
    pub login_failure: u64,
    pub attacks: u64,
    pub traces: u64,
    pub last_seen: i64,
    /// new_geolocation, many_failed_logins, …
    pub risks: Vec<String>,
}

#[derive(clickhouse::Row, Deserialize)]
struct UserRow {
    user: String,
    ips: Vec<String>,
    countries: Vec<String>,
    services: Vec<String>,
    ok: u64,
    fail: u64,
    attacks: u64,
    traces: u64,
    last: i64,
}

#[utoipa::path(post, path = "/api/v1/appsec/users", operation_id = "appsec_users", request_body = AppsecSearchRequest, responses((status = 200, body = Vec<AppsecUser>)), tag = "appsec")]
pub async fn users(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AppsecSearchRequest>) -> ApiResult<Vec<AppsecUser>> {
    let (w, p) = traces_where(req.range, &req.query);
    let rows: Vec<UserRow> = bind(
        st.ch(&ctx.org_id).query(&format!(
            "SELECT SpanAttributes['usr.id'] AS user, groupUniqArray(10)({IP}) AS ips, groupUniqArray(10)({COUNTRY}) AS countries, groupUniqArray(10)(ServiceName) AS services, \
             countIf({LOGIN_OK}) AS ok, countIf({LOGIN_FAIL}) AS fail, countIf({ATTACK}) AS attacks, count() AS traces, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND user != '' GROUP BY user ORDER BY fail DESC, traces DESC LIMIT 500"
        )),
        &p,
    )
    .fetch_all()
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let countries: Vec<String> = r.countries.into_iter().filter(|c| !c.is_empty()).collect();
                let mut risks = Vec::new();
                if countries.len() > 1 {
                    risks.push("new_geolocation".into());
                }
                if r.fail >= 5 && r.fail > r.ok {
                    risks.push("many_failed_logins".into());
                }
                if r.attacks > 0 {
                    risks.push("attacks".into());
                }
                AppsecUser { user: r.user, ips: r.ips.into_iter().filter(|i| !i.is_empty()).collect(), countries, services: r.services, login_success: r.ok, login_failure: r.fail, attacks: r.attacks, traces: r.traces, last_seen: r.last, risks }
            })
            .collect(),
    ))
}

// ================================================================ endpoints and services

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiEndpoint {
    pub service: String,
    pub env: String,
    pub method: String,
    pub route: String,
    pub hosts: Vec<String>,
    pub hits: u64,
    pub attacks: u64,
    pub blocked: u64,
    pub client_ips: u64,
    /// Requests with an authenticated user (`usr.id`).
    pub authenticated: u64,
    pub login_events: u64,
    /// The service runs App & API Protection.
    pub protected: bool,
    /// Response headers seen (the tracer reports them on security traces).
    pub headers_seen: bool,
    pub hsts: bool,
    pub frame_options: bool,
    pub server_errors_on_attacks: u64,
    pub last_seen: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct EndpointRow {
    service: String,
    env: String,
    method: String,
    route: String,
    hosts: Vec<String>,
    hits: u64,
    attacks: u64,
    blocked: u64,
    ips: u64,
    auth: u64,
    logins: u64,
    protected: u8,
    headers: u8,
    hsts: u8,
    xfo: u8,
    errs: u64,
    last: i64,
}

pub async fn endpoints_of(st: &AppState, org: &str, range: TimeRange) -> Result<Vec<ApiEndpoint>, ApiError> {
    let (rc, p) = range_clause(range);
    let rows: Vec<EndpointRow> = bind(
        st.ch(org).query(&format!(
            "SELECT ServiceName AS service, any({ENV}) AS env, {METHOD} AS method, {ROUTE} AS route, \
             groupUniqArray(5)(if(SpanAttributes['http.host'] != '', SpanAttributes['http.host'], SpanAttributes['server.address'])) AS hosts, count() AS hits, \
             countIf({ATTACK}) AS attacks, countIf({BLOCKED}) AS blocked, uniqExactIf({IP}, {IP} != '') AS ips, countIf(SpanAttributes['usr.id'] != '') AS auth, \
             countIf({LOGIN_OK} OR {LOGIN_FAIL}) AS logins, toUInt8(max({ENABLED})) AS protected, \
             toUInt8(max(SpanAttributes['http.response.headers.content-type'] != '')) AS headers, toUInt8(max(SpanAttributes['http.response.headers.strict-transport-security'] != '')) AS hsts, \
             toUInt8(max(SpanAttributes['http.response.headers.x-frame-options'] != '')) AS xfo, countIf({ATTACK} AND toUInt16OrZero({STATUS}) >= 500) AS errs, \
             toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {rc} AND {ENTRY} AND method != '' GROUP BY service, method, route ORDER BY hits DESC LIMIT 5000"
        )),
        &p,
    )
    .fetch_all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ApiEndpoint {
            service: r.service,
            env: r.env,
            method: r.method.to_uppercase(),
            route: r.route,
            hosts: r.hosts.into_iter().filter(|h| !h.is_empty()).collect(),
            hits: r.hits,
            attacks: r.attacks,
            blocked: r.blocked,
            client_ips: r.ips,
            authenticated: r.auth,
            login_events: r.logins,
            protected: r.protected == 1,
            headers_seen: r.headers == 1,
            hsts: r.hsts == 1,
            frame_options: r.xfo == 1,
            server_errors_on_attacks: r.errs,
            last_seen: r.last,
        })
        .collect())
}

/// An API finding rule: its OWASP API Top 10 2023 requirement and the check of an endpoint.
pub struct FindingRule {
    pub id: &'static str,
    pub name: &'static str,
    pub severity: &'static str,
    /// API1 … API10.
    pub owasp: &'static str,
    pub description: &'static str,
    pub remediation: &'static str,
    /// None: not applicable to the endpoint; Some(true): fails.
    pub check: fn(&ApiEndpoint) -> Option<bool>,
}

fn is_auth_route(e: &ApiEndpoint) -> bool {
    let r = e.route.to_lowercase();
    ["login", "signin", "sign-in", "auth", "token", "session"].iter().any(|k| r.contains(k)) && e.method == "POST"
}

fn writes(e: &ApiEndpoint) -> bool {
    matches!(e.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE")
}

pub const FINDING_RULES: &[FindingRule] = &[
    FindingRule {
        id: "oc-api-001",
        name: "Unauthenticated endpoint targeted by attacks",
        severity: "critical",
        owasp: "API2",
        description: "An endpoint that changes data receives attack payloads and none of its requests carries an authenticated user.",
        remediation: "Require authentication on the endpoint, or set the user on its spans (usr.id) if it already does.",
        check: |e| writes(e).then_some(e.attacks > 0 && e.authenticated == 0),
    },
    FindingRule {
        id: "oc-api-002",
        name: "Authentication route is not protected by account takeover detection",
        severity: "high",
        owasp: "API2",
        description: "A login route sends no login success or failure events, so credential stuffing and brute force go unnoticed.",
        remediation: "Track login events on the route (the tracers' user tracking SDK, or a custom In-App WAF rule).",
        check: |e| is_auth_route(e).then_some(e.login_events == 0),
    },
    FindingRule {
        id: "oc-api-003",
        name: "Endpoint exposed to attacks without blocking",
        severity: "high",
        owasp: "API8",
        description: "The endpoint receives attack payloads and none was blocked.",
        remediation: "Assign the service a blocking policy, or block the attacking IPs.",
        check: |e| (e.attacks > 0).then_some(e.blocked == 0),
    },
    FindingRule {
        id: "oc-api-004",
        name: "Endpoint returns server errors to attack payloads",
        severity: "medium",
        owasp: "API8",
        description: "Attack payloads make the endpoint fail with 5xx errors: input reaches code that doesn't expect it.",
        remediation: "Validate the input before it reaches the failing code, and look at the errors in APM.",
        check: |e| (e.attacks > 0).then_some(e.server_errors_on_attacks > 0),
    },
    FindingRule {
        id: "oc-api-005",
        name: "Endpoint not protected by App & API Protection",
        severity: "medium",
        owasp: "API9",
        description: "The service doesn't run App & API Protection, so attacks on the endpoint are neither detected nor blocked.",
        remediation: "Enable App & API Protection in the service's tracer (DD_APPSEC_ENABLED=true).",
        check: |e| Some(!e.protected),
    },
    FindingRule {
        id: "oc-api-006",
        name: "Missing Strict Transport Security HTTP header",
        severity: "low",
        owasp: "API8",
        description: "Responses don't set Strict-Transport-Security, so browsers may reach the API over plain HTTP.",
        remediation: "Set Strict-Transport-Security: max-age=31536000; includeSubDomains on responses.",
        check: |e| e.headers_seen.then_some(!e.hsts),
    },
    FindingRule {
        id: "oc-api-007",
        name: "Missing X-Frame-Options HTTP header",
        severity: "low",
        owasp: "API8",
        description: "Responses don't set X-Frame-Options, so pages can be framed by other sites (clickjacking).",
        remediation: "Set X-Frame-Options: DENY (or a frame-ancestors Content-Security-Policy).",
        check: |e| e.headers_seen.then_some(!e.frame_options),
    },
    FindingRule {
        id: "oc-api-008",
        name: "Endpoint scanned by many clients",
        severity: "info",
        owasp: "API4",
        description: "Attacks on the endpoint come from many client IPs: it is a known target.",
        remediation: "Rate limit the endpoint and review what it exposes.",
        check: |e| (e.attacks > 0).then_some(e.client_ips >= 10),
    },
];

/// The OWASP API Security Top 10 2023 requirements.
pub const OWASP: [(&str, &str); 10] = [
    ("API1", "Broken Object Level Authorization"),
    ("API2", "Broken Authentication"),
    ("API3", "Broken Object Property Level Authorization"),
    ("API4", "Unrestricted Resource Consumption"),
    ("API5", "Broken Function Level Authorization"),
    ("API6", "Unrestricted Access to Sensitive Business Flows"),
    ("API7", "Server Side Request Forgery"),
    ("API8", "Security Misconfiguration"),
    ("API9", "Improper Inventory Management"),
    ("API10", "Unsafe Consumption of APIs"),
];

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ApiFinding {
    pub key: String,
    pub rule_id: String,
    pub rule_name: String,
    pub severity: String,
    pub owasp: String,
    pub service: String,
    pub env: String,
    pub method: String,
    pub route: String,
    /// open | muted
    pub status: String,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FindingRuleStatus {
    pub id: String,
    pub name: String,
    pub severity: String,
    pub owasp: String,
    pub description: String,
    pub remediation: String,
    pub evaluated: usize,
    pub failing: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiFindingsPage {
    pub findings: Vec<ApiFinding>,
    pub rules: Vec<FindingRuleStatus>,
    pub owasp: Vec<NamedCount>,
}

pub fn evaluate_findings(endpoints: &[ApiEndpoint], triage: &HashMap<String, String>) -> (Vec<ApiFinding>, Vec<FindingRuleStatus>) {
    let mut findings = Vec::new();
    let mut rules = Vec::new();
    for r in FINDING_RULES {
        let (mut evaluated, mut failing) = (0, 0);
        for e in endpoints {
            let Some(fails) = (r.check)(e) else { continue };
            evaluated += 1;
            if !fails {
                continue;
            }
            failing += 1;
            let key = format!("api-finding:{}:{}:{} {}", r.id, e.service, e.method, e.route);
            findings.push(ApiFinding {
                status: triage.get(&key).cloned().unwrap_or_else(|| "open".into()),
                key,
                rule_id: r.id.into(),
                rule_name: r.name.into(),
                severity: r.severity.into(),
                owasp: r.owasp.into(),
                service: e.service.clone(),
                env: e.env.clone(),
                method: e.method.clone(),
                route: e.route.clone(),
            });
        }
        rules.push(FindingRuleStatus { id: r.id.into(), name: r.name.into(), severity: r.severity.into(), owasp: r.owasp.into(), description: r.description.into(), remediation: r.remediation.into(), evaluated, failing });
    }
    (findings, rules)
}

async fn findings_of(st: &AppState, org: &str, range: TimeRange) -> Result<(Vec<ApiEndpoint>, Vec<ApiFinding>, Vec<FindingRuleStatus>), ApiError> {
    let endpoints = endpoints_of(st, org, range).await?;
    let triage = super::code_security::triage_map(st, org).await?;
    let (f, r) = evaluate_findings(&endpoints, &triage);
    Ok((endpoints, f, r))
}

fn week() -> TimeRange {
    let now = chrono::Utc::now().timestamp_millis();
    TimeRange { from: now - 7 * DAY, to: now }
}

#[utoipa::path(get, path = "/api/v1/appsec/findings", operation_id = "appsec_api_findings", responses((status = 200, body = ApiFindingsPage)), tag = "appsec")]
pub async fn findings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<ApiFindingsPage> {
    let (_, findings, rules) = findings_of(&st, &ctx.org_id, week()).await?;
    let owasp = OWASP.iter().map(|(id, _)| NamedCount { name: id.to_string(), count: findings.iter().filter(|f| f.owasp == *id && f.status == "open").count() as u64 }).collect();
    Ok(Json(ApiFindingsPage { findings, rules, owasp }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndpointPage {
    pub endpoints: Vec<ApiEndpointView>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiEndpointView {
    #[serde(flatten)]
    pub endpoint: ApiEndpoint,
    /// Open findings by severity.
    pub findings: BTreeMap<String, usize>,
}

#[utoipa::path(get, path = "/api/v1/appsec/endpoints", operation_id = "appsec_endpoints", responses((status = 200, body = EndpointPage)), tag = "appsec")]
pub async fn endpoints(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<EndpointPage> {
    let (endpoints, findings, _) = findings_of(&st, &ctx.org_id, week()).await?;
    let mut by: HashMap<String, BTreeMap<String, usize>> = HashMap::new();
    for f in findings.iter().filter(|f| f.status == "open") {
        *by.entry(format!("{}|{}|{}", f.service, f.method, f.route)).or_default().entry(f.severity.clone()).or_default() += 1;
    }
    Ok(Json(EndpointPage {
        endpoints: endpoints
            .into_iter()
            .map(|e| {
                let findings = by.remove(&format!("{}|{}|{}", e.service, e.method, e.route)).unwrap_or_default();
                ApiEndpointView { endpoint: e, findings }
            })
            .collect(),
    }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppsecService {
    pub service: String,
    pub env: String,
    pub language: String,
    pub protected: bool,
    /// monitoring | blocking | disabled (the WAF policy assigned to the service).
    pub mode: String,
    pub policy: String,
    pub security_traces: u64,
    /// Security traces per day over the last 7 days.
    pub trend: Vec<u64>,
    /// Library vulnerabilities (Code Security).
    pub vulnerabilities: usize,
    pub sca: bool,
    pub iast: bool,
}

#[derive(clickhouse::Row, Deserialize)]
struct ServiceRow {
    service: String,
    env: String,
    lang: String,
    protected: u8,
    sec: u64,
    days: Vec<u64>,
}

/// The policy of a service: an org policy whose scope names it, else the default.
fn policy_of(service: &str, env: &str, policies: &[waf::WafRow]) -> (String, String) {
    for p in policies {
        let scoped = p.attributes.get("scope").and_then(Value::as_array).is_some_and(|s| {
            s.iter().any(|x| x.get("service").and_then(Value::as_str).is_none_or(|v| v == service) && x.get("env").and_then(Value::as_str).is_none_or(|v| v.is_empty() || v == env))
        });
        if scoped {
            let blocking = p.attributes.get("rules").and_then(Value::as_array).is_some_and(|r| r.iter().any(|x| x.get("blocking").and_then(Value::as_bool) == Some(true)))
                || p.attributes.get("protectionPresets").and_then(Value::as_array).is_some_and(|x| !x.is_empty());
            return (p.attributes.get("name").and_then(Value::as_str).unwrap_or("Custom policy").to_string(), if blocking { "blocking" } else { "monitoring" }.into());
        }
    }
    (MANAGED_POLICIES[0].1.to_string(), "monitoring".into())
}

pub async fn services_of(st: &AppState, org: &str) -> Result<Vec<AppsecService>, ApiError> {
    let now = chrono::Utc::now().timestamp_millis();
    let start = now - now.rem_euclid(DAY) - 6 * DAY;
    let rows: Vec<ServiceRow> = st
        .ch(org)
        .query(&format!(
            "SELECT ServiceName AS service, any({ENV}) AS env, any(ResourceAttributes['telemetry.sdk.language']) AS lang, toUInt8(max({ENABLED})) AS protected, \
             countIf({}) AS sec, arrayMap(d -> countIf({} AND toInt64(toUnixTimestamp64Milli(Timestamp)) >= {start} + d * {DAY} AND toInt64(toUnixTimestamp64Milli(Timestamp)) < {start} + (d + 1) * {DAY}), range(7)) AS days \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= now() - INTERVAL 7 DAY AND {ENTRY} GROUP BY service ORDER BY sec DESC, service",
            security(),
            security()
        ))
        .fetch_all()
        .await?;
    let policies = waf::list_rows(st, org, Kind::Policy).await?;
    let vulns = super::code_security::list_library_vulnerabilities(State(st.clone()), crate::state::OrgId(org.to_string()), Json(super::code_security::CsFilter { limit: 5000, ..Default::default() })).await?.0.vulnerabilities;
    let setup_iast: Vec<String> = sqlx::query_scalar("SELECT DISTINCT service FROM cs_libraries WHERE org_id = $1 AND source = 'runtime'").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let (policy, mode) = if r.protected == 1 { policy_of(&r.service, &r.env, &policies) } else { (String::new(), "disabled".into()) };
            AppsecService {
                vulnerabilities: vulns.iter().filter(|v| v.services.contains(&r.service)).count(),
                sca: setup_iast.contains(&r.service),
                iast: false,
                service: r.service,
                env: r.env,
                language: r.lang,
                protected: r.protected == 1,
                mode,
                policy,
                security_traces: r.sec,
                trend: r.days,
            }
        })
        .collect())
}

#[utoipa::path(get, path = "/api/v1/appsec/services", operation_id = "appsec_services", responses((status = 200, body = Vec<AppsecService>)), tag = "appsec")]
pub async fn services(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AppsecService>> {
    Ok(Json(services_of(&st, &ctx.org_id).await?))
}

// ================================================================ overview

#[derive(Deserialize, ToSchema)]
pub struct AppsecOverviewRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrendCount {
    pub name: String,
    pub count: u64,
    /// Change against the previous period, in percent (None: new).
    pub change: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppsecOverview {
    pub endpoints: usize,
    pub endpoints_with_traffic: usize,
    pub unprotected_endpoints: usize,
    pub authenticated_endpoints: usize,
    pub unauthenticated_endpoints: usize,
    pub exposed_endpoints: usize,
    /// Open findings by severity.
    pub findings: BTreeMap<String, usize>,
    pub top_finding_rules: Vec<NamedCount>,
    /// Endpoints with the most open findings: `service|METHOD route` → count.
    pub threat_endpoints: Vec<NamedCount>,
    pub finding_rules_enabled: usize,
    pub owasp_covered: Vec<String>,
    /// Services running App & API Protection, of those with traffic.
    pub services_protected: usize,
    pub services_total: usize,
    pub services_blocking: usize,
    pub services_monitoring: usize,
    pub services_with_attacks: Vec<NamedCount>,
    pub top_attack_types: Vec<TrendCount>,
    pub top_countries: Vec<TrendCount>,
}

async fn trend(st: &AppState, org: &str, expr: &str, range: TimeRange) -> Result<Vec<TrendCount>, ApiError> {
    let len = range.to - range.from;
    let (w, p) = traces_where(range, "");
    let (wp, pp) = traces_where(TimeRange { from: range.from - len, to: range.from }, "");
    let now = kv(st, org, format!("SELECT toString({expr}) AS k, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND {ATTACK} GROUP BY k HAVING k != '' ORDER BY n DESC LIMIT 5"), &p).await?;
    let before: HashMap<String, u64> = kv(st, org, format!("SELECT toString({expr}) AS k, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {wp} AND {ATTACK} GROUP BY k HAVING k != ''"), &pp).await?.into_iter().map(|r| (r.k, r.n)).collect();
    Ok(now
        .into_iter()
        .map(|r| {
            let change = before.get(&r.k).filter(|b| **b > 0).map(|b| ((r.n as f64 - *b as f64) / *b as f64 * 100.0).round());
            TrendCount { name: r.k, count: r.n, change }
        })
        .collect())
}

#[utoipa::path(post, path = "/api/v1/appsec/overview", operation_id = "appsec_overview", request_body = AppsecOverviewRequest, responses((status = 200, body = AppsecOverview)), tag = "appsec")]
pub async fn overview(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AppsecOverviewRequest>) -> ApiResult<AppsecOverview> {
    let org = &ctx.org_id;
    let (endpoints, findings, rules) = findings_of(&st, org, req.range).await?;
    let open: Vec<&ApiFinding> = findings.iter().filter(|f| f.status == "open").collect();
    let mut by_sev: BTreeMap<String, usize> = ["critical", "high", "medium", "low", "info"].iter().map(|s| (s.to_string(), 0)).collect();
    for f in &open {
        *by_sev.entry(f.severity.clone()).or_default() += 1;
    }
    let rank = |s: &str| ["info", "low", "medium", "high", "critical"].iter().position(|x| *x == s).unwrap_or(0);
    let mut top_rules: Vec<(String, String, usize)> = rules.iter().filter(|r| r.failing > 0).map(|r| (r.name.clone(), r.severity.clone(), r.failing)).collect();
    top_rules.sort_by(|a, b| rank(&b.1).cmp(&rank(&a.1)).then(b.2.cmp(&a.2)));
    let mut by_ep: HashMap<String, usize> = HashMap::new();
    for f in open.iter().filter(|f| matches!(f.severity.as_str(), "critical" | "high")) {
        *by_ep.entry(format!("{}|{} {}", f.service, f.method, f.route)).or_default() += 1;
    }
    let mut threat_endpoints: Vec<NamedCount> = by_ep.into_iter().map(|(name, n)| NamedCount { name, count: n as u64 }).collect();
    threat_endpoints.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    threat_endpoints.truncate(5);
    let services = services_of(&st, org).await?;
    let mut with_attacks: Vec<NamedCount> = services.iter().filter(|s| s.security_traces > 0).map(|s| NamedCount { name: s.service.clone(), count: s.security_traces }).collect();
    with_attacks.truncate(5);
    Ok(Json(AppsecOverview {
        endpoints: endpoints.len(),
        endpoints_with_traffic: endpoints.iter().filter(|e| e.hits > 0).count(),
        unprotected_endpoints: endpoints.iter().filter(|e| !e.protected).count(),
        authenticated_endpoints: endpoints.iter().filter(|e| e.authenticated > 0).count(),
        unauthenticated_endpoints: endpoints.iter().filter(|e| e.authenticated == 0 && writes(e)).count(),
        exposed_endpoints: endpoints.iter().filter(|e| e.attacks > 0).count(),
        findings: by_sev,
        top_finding_rules: top_rules.into_iter().take(5).map(|(n, _, c)| NamedCount { name: n, count: c as u64 }).collect(),
        threat_endpoints,
        finding_rules_enabled: FINDING_RULES.len(),
        owasp_covered: FINDING_RULES.iter().map(|r| r.owasp.to_string()).collect::<BTreeSet<_>>().into_iter().collect(),
        services_protected: services.iter().filter(|s| s.protected).count(),
        services_total: services.len(),
        services_blocking: services.iter().filter(|s| s.mode == "blocking").count(),
        services_monitoring: services.iter().filter(|s| s.mode == "monitoring").count(),
        services_with_attacks: with_attacks,
        top_attack_types: trend(&st, org, &format!("arrayJoin(if(empty({TYPES}), [''], {TYPES}))"), req.range).await?,
        top_countries: trend(&st, org, COUNTRY, req.range).await?,
    }))
}

// ================================================================ business logic

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BusinessEvent {
    pub event: String,
    pub description: String,
    pub requests7d: u64,
    pub services: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedRule {
    pub service: String,
    pub env: String,
    pub route: String,
    /// users.login.success | users.login.failure
    pub event: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BusinessLogic {
    pub events: Vec<BusinessEvent>,
    pub suggested: Vec<SuggestedRule>,
}

const BUSINESS_EVENTS: [(&str, &str); 8] = [
    ("users.login.failure", "Tracks failed user logins."),
    ("users.login.success", "Tracks successful user logins."),
    ("users.signup", "Tracks user signups."),
    ("users.delete", "Tracks user deletions."),
    ("users.password_reset", "Tracks user password resets."),
    ("activity.sensitive", "Tracks any sensitive activity in the application."),
    ("payment.attempt", "Tracks payment attempts."),
    ("payment.failure", "Tracks failed payment attempts."),
];

#[derive(clickhouse::Row, Deserialize)]
struct EventRow {
    event: String,
    n: u64,
    services: Vec<String>,
}

#[utoipa::path(get, path = "/api/v1/appsec/business-logic", operation_id = "appsec_business_logic", responses((status = 200, body = BusinessLogic)), tag = "appsec")]
pub async fn business_logic(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<BusinessLogic> {
    let rows: Vec<EventRow> = st
        .ch(&ctx.org_id)
        .query(
            "SELECT replaceRegexpOne(replaceRegexpOne(k, '^appsec\\\\.events\\\\.', ''), '\\\\.track$', '') AS event, count() AS n, groupUniqArray(10)(ServiceName) AS services \
             FROM otel.otel_traces ARRAY JOIN mapKeys(SpanAttributes) AS k WHERE OrgId = {oc_org:String} AND Timestamp >= now() - INTERVAL 7 DAY \
             AND startsWith(k, 'appsec.events.') AND endsWith(k, '.track') GROUP BY event",
        )
        .fetch_all()
        .await?;
    let by: HashMap<String, EventRow> = rows.into_iter().map(|r| (r.event.clone(), r)).collect();
    let events = BUSINESS_EVENTS
        .iter()
        .map(|(e, d)| BusinessEvent { event: e.to_string(), description: d.to_string(), requests7d: by.get(*e).map_or(0, |r| r.n), services: by.get(*e).map(|r| r.services.clone()).unwrap_or_default() })
        .collect();
    let eps = endpoints_of(&st, &ctx.org_id, week()).await?;
    let suggested = eps
        .iter()
        .filter(|e| is_auth_route(e) && e.login_events == 0)
        .flat_map(|e| ["users.login.success", "users.login.failure"].map(|ev| SuggestedRule { service: e.service.clone(), env: e.env.clone(), route: e.route.clone(), event: ev.into() }))
        .collect();
    Ok(Json(BusinessLogic { events, suggested }))
}

// ================================================================ denylist

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DenylistEntry {
    /// ip | user
    pub kind: String,
    pub value: String,
    pub reason: String,
    pub source: String,
    /// Epoch ms; null: permanent.
    pub expires_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DenylistView {
    #[serde(flatten)]
    pub entry: DenylistEntry,
    /// Security traces from the entity over the last 10 days, per day.
    pub activity: Vec<u64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DenylistInput {
    /// ip | user
    pub kind: String,
    pub values: Vec<String>,
    #[serde(default)]
    pub reason: String,
    /// Minutes; null: permanent.
    pub duration_minutes: Option<i64>,
}

#[derive(clickhouse::Row, Deserialize)]
struct ActRow {
    ip: String,
    days: Vec<u64>,
}

#[utoipa::path(get, path = "/api/v1/appsec/denylist", operation_id = "appsec_denylist", responses((status = 200, body = Vec<DenylistView>)), tag = "appsec")]
pub async fn denylist(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<DenylistView>> {
    let rows: Vec<DenylistEntry> = sqlx::query_as(
        "SELECT kind, value, reason, source, (extract(epoch FROM expires_at) * 1000)::bigint AS expires_at, created_by, (extract(epoch FROM created_at) * 1000)::bigint AS created_at \
         FROM appsec_denylist WHERE org_id = $1 AND (expires_at IS NULL OR expires_at > now()) ORDER BY created_at DESC",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    let now = chrono::Utc::now().timestamp_millis();
    let start = now - now.rem_euclid(DAY) - 9 * DAY;
    let ips: Vec<String> = rows.iter().filter(|r| r.kind == "ip").map(|r| r.value.clone()).collect();
    let act: HashMap<String, Vec<u64>> = if ips.is_empty() {
        HashMap::new()
    } else {
        st.ch(&ctx.org_id)
            .query(&format!(
                "SELECT {IP} AS ip, arrayMap(d -> countIf(toInt64(toUnixTimestamp64Milli(Timestamp)) >= {start} + d * {DAY} AND toInt64(toUnixTimestamp64Milli(Timestamp)) < {start} + (d + 1) * {DAY}), range(10)) AS days \
                 FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND {ENTRY} AND {} AND ip IN ? GROUP BY ip",
                security()
            ))
            .bind(start)
            .bind(&ips)
            .fetch_all::<ActRow>()
            .await?
            .into_iter()
            .map(|r| (r.ip, r.days))
            .collect()
    };
    Ok(Json(rows.into_iter().map(|e| DenylistView { activity: act.get(&e.value).cloned().unwrap_or_else(|| vec![0; 10]), entry: e }).collect()))
}

#[utoipa::path(post, path = "/api/v1/appsec/denylist", operation_id = "appsec_block", request_body = DenylistInput, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "appsec")]
pub async fn block(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<DenylistInput>) -> Result<StatusCode, ApiError> {
    if !matches!(input.kind.as_str(), "ip" | "user") || input.values.is_empty() || input.values.len() > 500 {
        return Err(ApiError::BadRequest("block between 1 and 500 IPs or users".into()));
    }
    for v in &input.values {
        let v = v.trim();
        if input.kind == "ip" && v.parse::<std::net::IpAddr>().is_err() {
            return Err(ApiError::BadRequest(format!("{v} is not an IP address")));
        }
        sqlx::query(
            "INSERT INTO appsec_denylist (org_id, kind, value, reason, source, expires_at, created_by) VALUES ($1, $2, $3, $4, 'Attackers Explorer', \
             CASE WHEN $5::bigint IS NULL THEN NULL ELSE now() + make_interval(mins => $5::int) END, $6) \
             ON CONFLICT (org_id, kind, value) DO UPDATE SET reason = $4, expires_at = EXCLUDED.expires_at, created_by = $6, created_at = now()",
        )
        .bind(&ctx.org_id)
        .bind(&input.kind)
        .bind(v)
        .bind(&input.reason)
        .bind(input.duration_minutes)
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/appsec/denylist/{kind}/{value}", operation_id = "appsec_unblock", params(("kind" = String, Path), ("value" = String, Path)), responses((status = 204)), tag = "appsec")]
pub async fn unblock(State(st): State<AppState>, Writer(ctx): Writer, Path((kind, value)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM appsec_denylist WHERE org_id = $1 AND kind = $2 AND value = $3").bind(&ctx.org_id).bind(&kind).bind(&value).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ WAF configuration for the UI

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WafPolicyView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub managed: bool,
    pub is_default: bool,
    pub blocking: usize,
    pub monitoring: usize,
    pub disabled: usize,
    pub services: Vec<String>,
    pub modified_by: String,
}

/// The In-App WAF rule set's size: (blocking, monitoring, disabled) per managed policy.
const RULESET: [(usize, usize, usize); 3] = [(0, 186, 33), (47, 139, 33), (117, 69, 33)];

fn kind_of(s: &str) -> Result<Kind, ApiError> {
    match s {
        "custom_rule" => Ok(Kind::CustomRule),
        "exclusion_filter" => Ok(Kind::ExclusionFilter),
        "policy" => Ok(Kind::Policy),
        _ => Err(ApiError::NotFound(format!("unknown WAF object kind {s}"))),
    }
}

#[utoipa::path(get, path = "/api/v1/appsec/policies", operation_id = "appsec_waf_policies", responses((status = 200, body = Vec<WafPolicyView>)), tag = "appsec")]
pub async fn policies(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<WafPolicyView>> {
    let rows = waf::list_rows(&st, &ctx.org_id, Kind::Policy).await?;
    let services = services_of(&st, &ctx.org_id).await?;
    let mut out: Vec<WafPolicyView> = MANAGED_POLICIES
        .iter()
        .zip(RULESET)
        .map(|((id, name, desc, default), (b, m, d))| WafPolicyView {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            managed: true,
            is_default: *default,
            blocking: b,
            monitoring: m,
            disabled: d,
            services: if *default { services.iter().filter(|s| s.protected && s.policy == *name).map(|s| s.service.clone()).collect() } else { vec![] },
            modified_by: "ObserveCraft".into(),
        })
        .collect();
    for r in rows {
        let a = &r.attributes;
        let presets: Vec<&str> = a.get("protectionPresets").and_then(Value::as_array).map(|x| x.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let (b, m, d) = if presets.contains(&"application-attacks") { RULESET[2] } else if presets.contains(&"attack-tools") { RULESET[1] } else { RULESET[0] };
        let name = a.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        out.push(WafPolicyView {
            id: r.id.clone(),
            description: a.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
            managed: false,
            is_default: false,
            blocking: b,
            monitoring: m,
            disabled: d,
            services: services.iter().filter(|s| s.policy == name).map(|s| s.service.clone()).collect(),
            modified_by: r.modified_by_name.clone().unwrap_or(r.added_by_name.clone()),
            name,
        });
    }
    Ok(Json(out))
}

#[utoipa::path(get, path = "/api/v1/appsec/waf/{kind}", operation_id = "appsec_waf_list", params(("kind" = String, Path)), responses((status = 200, body = Vec<Object>)), tag = "appsec")]
pub async fn waf_list(State(st): State<AppState>, ctx: AuthContext, Path(kind): Path<String>) -> ApiResult<Vec<Value>> {
    let k = kind_of(&kind)?;
    Ok(Json(waf::list_rows(&st, &ctx.org_id, k).await?.iter().map(|r| waf::doc(k, r)).collect()))
}

fn ui_attributes(k: Kind, body: Value) -> Result<Map<String, Value>, ApiError> {
    let a = body.as_object().cloned().ok_or_else(|| ApiError::BadRequest("the body must be the object's attributes".into()))?;
    waf::check(k, &a).map_err(|m| ApiError::BadRequest(m.into()))?;
    Ok(a)
}

#[utoipa::path(post, path = "/api/v1/appsec/waf/{kind}", operation_id = "appsec_waf_create", params(("kind" = String, Path)), request_body = Object, responses((status = 200, body = Object)), tag = "appsec")]
pub async fn waf_create(State(st): State<AppState>, Writer(ctx): Writer, Path(kind): Path<String>, Json(body): Json<Value>) -> ApiResult<Value> {
    let k = kind_of(&kind)?;
    let a = ui_attributes(k, body)?;
    Ok(Json(waf::doc(k, &waf::insert(&st, &ctx, k, &a).await?)))
}

#[utoipa::path(put, path = "/api/v1/appsec/waf/{kind}/{id}", operation_id = "appsec_waf_update", params(("kind" = String, Path), ("id" = String, Path)), request_body = Object, responses((status = 200, body = Object)), tag = "appsec")]
pub async fn waf_update(State(st): State<AppState>, Writer(ctx): Writer, Path((kind, id)): Path<(String, String)>, Json(body): Json<Value>) -> ApiResult<Value> {
    let k = kind_of(&kind)?;
    let a = ui_attributes(k, body)?;
    let row = waf::replace(&st, &ctx, k, &id, &a).await?.ok_or_else(|| ApiError::NotFound(format!("no {kind} {id}")))?;
    Ok(Json(waf::doc(k, &row)))
}

#[utoipa::path(delete, path = "/api/v1/appsec/waf/{kind}/{id}", operation_id = "appsec_waf_delete", params(("kind" = String, Path), ("id" = String, Path)), responses((status = 204)), tag = "appsec")]
pub async fn waf_delete(State(st): State<AppState>, Writer(ctx): Writer, Path((kind, id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    let k = kind_of(&kind)?;
    if !waf::remove(&st, &ctx.org_id, k, &id).await? {
        return Err(ApiError::NotFound(format!("no {kind} {id}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ settings

#[utoipa::path(get, path = "/api/v1/appsec/settings/{key}", operation_id = "appsec_setting", params(("key" = String, Path)), responses((status = 200, body = Object)), tag = "appsec")]
pub async fn setting(State(st): State<AppState>, ctx: AuthContext, Path(key): Path<String>) -> ApiResult<Value> {
    let v: Option<Value> = sqlx::query_scalar("SELECT value FROM appsec_settings WHERE org_id = $1 AND key = $2").bind(&ctx.org_id).bind(&key).fetch_optional(&st.postgres).await?;
    Ok(Json(v.unwrap_or_else(|| default_setting(&key))))
}

fn default_setting(key: &str) -> Value {
    match key {
        "custom_response" => json!({ "mode": "tracer", "status": 403, "redirectUrl": "", "redirectStatus": 302 }),
        "blocking_allowed" => json!(true),
        _ => json!({}),
    }
}

#[utoipa::path(put, path = "/api/v1/appsec/settings/{key}", operation_id = "appsec_set_setting", params(("key" = String, Path)), request_body = Object, responses((status = 204)), tag = "appsec")]
pub async fn set_setting(State(st): State<AppState>, Writer(ctx): Writer, Path(key): Path<String>, Json(v): Json<Value>) -> Result<StatusCode, ApiError> {
    if !matches!(key.as_str(), "custom_response" | "blocking_allowed" | "scanners") {
        return Err(ApiError::NotFound(format!("unknown setting {key}")));
    }
    sqlx::query("INSERT INTO appsec_settings (org_id, key, value) VALUES ($1, $2, $3) ON CONFLICT (org_id, key) DO UPDATE SET value = $3, updated_at = now()")
        .bind(&ctx.org_id)
        .bind(&key)
        .bind(&v)
        .execute(&st.postgres)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ detection rules

/// A default detection rule: (id, name, message, query, group by, cases (condition, severity, name), window).
pub type DefaultRule = (&'static str, &'static str, &'static str, &'static str, &'static [&'static str], &'static [(&'static str, &'static str, &'static str)], i64);

/// Default detection rules on the security traces.
pub const DEFAULT_RULES: [DefaultRule; 4] = [
    ("def-aap-001", "Attack attempts on a service", "A burst of attack attempts reached the service. Look at the attacking IPs and whether any request succeeded.", "@appsec.category:attack_attempt", &["service"], &[("a > 1000", "medium", "Condition 2"), ("a > 100", "info", "Condition 1")], 900),
    ("def-aap-002", "Security scanner detected", "A security scanner is probing the service from one IP.", "@appsec.type:security_scanner", &["@http.client_ip"], &[("a > 50", "low", "")], 900),
    ("def-aap-003", "Credential stuffing attack on a login route", "Many failed logins from one IP: a credential stuffing or brute force attempt.", "@appsec.events.users.login.failure.track:true", &["@http.client_ip"], &[("a > 20", "high", "")], 900),
    ("def-aap-004", "SQL injection attempts on a service", "SQL injection payloads reached the service.", "@appsec.type:sql_injection", &["service"], &[("a > 10", "low", "")], 900),
];

#[derive(clickhouse::Row, Deserialize)]
struct GroupRow {
    g: Vec<String>,
    n: u64,
}

fn group_expr(key: &str, params: &mut Vec<Param>) -> String {
    let (e, p) = field(key);
    params.extend(p);
    format!("toString({e})")
}

/// Per group, the security traces matching `query` over `range` (the engine's `application_security` rules).
pub async fn group_counts(st: &AppState, org: &str, query: &str, group_by: &[String], range: TimeRange) -> Result<Vec<(Vec<String>, f64)>, ApiError> {
    let mut params = Vec::new();
    let exprs: Vec<String> = group_by.iter().map(|k| group_expr(k, &mut params)).collect();
    let g = if exprs.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", exprs.join(", ")) };
    let (w, p) = traces_where(range, query);
    params.extend(p);
    let rows: Vec<GroupRow> = bind(st.ch(org).query(&format!("SELECT {g} AS g, count() AS n FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} GROUP BY g ORDER BY n DESC LIMIT 1000")), &params).fetch_all().await?;
    Ok(rows.into_iter().map(|r| (r.g, r.n as f64)).collect())
}

#[derive(clickhouse::Row, Deserialize)]
struct EntityRow {
    n: u64,
    ips: Vec<String>,
    ip_count: u64,
    countries: Vec<String>,
    country_count: u64,
    users: Vec<String>,
    user_count: u64,
    services: Vec<String>,
    first: i64,
}

/// The matching security traces' count, top IPs, countries, users and services, as signal entities.
pub async fn entities(st: &AppState, org: &str, query: &str, group: &BTreeMap<String, String>, range: TimeRange) -> Result<(u64, Value, i64), ApiError> {
    let (mut w, mut p) = traces_where(range, query);
    for (k, v) in group {
        let (e, kp) = field(k);
        p.extend(kp);
        w.push_str(&format!(" AND toString({e}) = ?"));
        p.push(Param::Str(v.clone()));
    }
    let r: EntityRow = bind(
        st.ch(org).query(&format!(
            "SELECT count() AS n, topKIf(5)({IP}, {IP} != '') AS ips, toUInt64(uniqExactIf({IP}, {IP} != '')) AS ip_count, topKIf(5)({COUNTRY}, {COUNTRY} != '') AS countries, \
             toUInt64(uniqExactIf({COUNTRY}, {COUNTRY} != '')) AS country_count, topKIf(5)(SpanAttributes['usr.id'], SpanAttributes['usr.id'] != '') AS users, \
             toUInt64(uniqExactIf(SpanAttributes['usr.id'], SpanAttributes['usr.id'] != '')) AS user_count, topK(3)(ServiceName) AS services, \
             toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w}"
        )),
        &p,
    )
    .fetch_one()
    .await?;
    let mut out = Map::new();
    for (k, top, count) in [("ip", r.ips, r.ip_count), ("country", r.countries, r.country_count), ("user", r.users, r.user_count), ("service", r.services.clone(), r.services.len() as u64)] {
        if count > 0 {
            out.insert(k.into(), json!({ "top": top, "count": count }));
        }
    }
    Ok((r.n, Value::Object(out), r.first))
}

// ================================================================ demo

/// The demo org's denylist (the IP oc-tracegen's blocked attacks come from), a passlist entry for the
/// office scanner and a custom rule. Idempotent.
pub async fn seed_demo(st: &AppState, org: &str) -> Result<(), ApiError> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM appsec_waf_objects WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await?;
    if n > 0 {
        return Ok(());
    }
    for (ip, reason, src) in [("203.0.113.41", "Repeated SQL injection attempts", "Attack attempts on a service"), ("198.51.100.201", "Credential stuffing", "Attackers Explorer")] {
        sqlx::query("INSERT INTO appsec_denylist (org_id, kind, value, reason, source, created_by) VALUES ($1, 'ip', $2, $3, $4, 'Priya Shah') ON CONFLICT DO NOTHING")
            .bind(org)
            .bind(ip)
            .bind(reason)
            .bind(src)
            .execute(&st.postgres)
            .await?;
    }
    for (kind, attrs) in [
        ("exclusion_filter", json!({ "description": "Internal vulnerability scanner (weekly scan)", "enabled": true, "ip_list": ["192.0.2.250"], "on_match": "monitor" })),
        ("exclusion_filter", json!({ "description": "Search accepts code snippets", "enabled": true, "path_glob": "/search", "parameters": ["q"], "rules_target": [{ "tags": { "category": "attack_attempt", "type": "xss" } }], "scope": [{ "env": "prod", "service": "storefront-api" }] })),
        ("custom_rule", json!({ "name": "Track users.login.failure on api-gateway - /api/v1/login", "enabled": true, "blocking": false, "path_glob": "/api/v1/login", "scope": [{ "env": "prod", "service": "api-gateway" }],
            "conditions": [{ "operator": "match_regex", "parameters": { "inputs": [{ "address": "server.response.status" }], "regex": "^401$", "options": {} } }], "tags": { "category": "business_logic", "type": "users.login.failure" } })),
    ] {
        sqlx::query("INSERT INTO appsec_waf_objects (org_id, kind, attributes, added_by, added_by_name) VALUES ($1, $2, $3, '', 'Priya Shah')").bind(org).bind(kind).bind(attrs).execute(&st.postgres).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(method: &str, route: &str) -> ApiEndpoint {
        ApiEndpoint {
            service: "api-gateway".into(),
            env: "prod".into(),
            method: method.into(),
            route: route.into(),
            hosts: vec![],
            hits: 100,
            attacks: 0,
            blocked: 0,
            client_ips: 1,
            authenticated: 0,
            login_events: 0,
            protected: true,
            headers_seen: false,
            hsts: false,
            frame_options: false,
            server_errors_on_attacks: 0,
            last_seen: 0,
        }
    }

    #[test]
    fn triggers_parse_datadogs_appsec_json() {
        let j = r#"{"triggers":[{"rule":{"id":"crs-942-100","name":"SQL Injection Attack","tags":{"type":"sql_injection","category":"attack_attempt"}},
                    "rule_matches":[{"operator":"is_sqli","parameters":[{"address":"server.request.query","key_path":["id"],"value":"1' OR '1'='1"}]}]}]}"#;
        let t = triggers(j);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].kind.as_str(), t[0].category.as_str()), ("sql_injection", "attack_attempt"));
        assert_eq!(t[0].matches, ["server.request.query.id: 1' OR '1'='1"]);
        assert!(triggers("not json").is_empty());
    }

    #[test]
    fn finding_rules_check_endpoints() {
        let mut login = ep("POST", "/api/v1/login");
        let triage = HashMap::new();
        let (f, _) = evaluate_findings(std::slice::from_ref(&login), &triage);
        assert!(f.iter().any(|x| x.rule_id == "oc-api-002"));
        login.login_events = 10;
        login.attacks = 5;
        login.authenticated = 0;
        let (f, rules) = evaluate_findings(std::slice::from_ref(&login), &triage);
        let ids: Vec<&str> = f.iter().map(|x| x.rule_id.as_str()).collect();
        assert!(ids.contains(&"oc-api-001") && ids.contains(&"oc-api-003") && !ids.contains(&"oc-api-002"));
        assert_eq!(rules.iter().find(|r| r.id == "oc-api-006").unwrap().evaluated, 0);
    }

    #[test]
    fn searches_map_security_fields() {
        let (c, p) = search_clause("@appsec.type:sql_injection service:api-gateway -@http.client_ip:198.51.100.*");
        assert!(c.contains("has(") && c.contains("NOT (") && c.contains("ServiceName"), "{c}");
        assert_eq!(p.len(), 3);
    }

    /// Security traces become attackers, endpoints, findings and rule counts. Live: skipped unless
    /// OC_TEST_CLICKHOUSE_URL and OC_TEST_POSTGRES_URL are set.
    #[tokio::test]
    async fn spans_become_security_traces() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "aap").await;
        let now = chrono::Utc::now().timestamp_millis();
        let sqli = r#"{"triggers":[{"rule":{"id":"oc-942-100","name":"SQL injection attempt","tags":{"type":"sql_injection","category":"attack_attempt"}},"rule_matches":[]}]}"#;
        st.ch(&org)
            .query(&format!(
                "INSERT INTO otel.otel_traces (Timestamp, TraceId, SpanId, SpanName, SpanKind, ServiceName, ResourceAttributes, SpanAttributes, Duration, StatusCode) \
                 SELECT fromUnixTimestamp64Milli(toInt64({now} - 60000 + number * 100)), lower(hex(number + 1)), lower(hex(number + 1)), 'POST /api/orders', 'Server', 'api-gateway', \
                        map('deployment.environment', 'prod', 'oc.org_id', '{org}'), \
                        if(number < 30, map('http.method', 'POST', 'http.route', '/api/orders', 'http.client_ip', '203.0.113.7', 'http.status_code', '400', '_dd.appsec.enabled', '1', '_dd.appsec.json', '{sqli}', 'appsec.event', 'true'), \
                           map('http.method', 'POST', 'http.route', '/api/orders', 'http.client_ip', '192.0.2.31', 'http.status_code', '200', '_dd.appsec.enabled', '1')), 1000000, 'Ok' \
                 FROM numbers(50)"
            ))
            .execute()
            .await
            .expect("insert spans");
        let range = TimeRange { from: now - 3_600_000, to: now + 60_000 };
        let page = search_traces(st, &org, &AppsecSearchRequest { range, query: "@appsec.type:sql_injection".into(), limit: None }).await.unwrap();
        assert_eq!(page.total, 30);
        assert_eq!(page.traces[0].triggers[0].kind, "sql_injection");
        assert!(page.top_activities.iter().any(|a| a.name == "attack_attempt.sql_injection" && a.count == 30));
        let attackers = attackers_of(st, &org, range, "").await.unwrap();
        assert_eq!(attackers.len(), 1);
        assert_eq!((attackers[0].ip.as_str(), attackers[0].attacks), ("203.0.113.7", 30));
        let eps = endpoints_of(st, &org, range).await.unwrap();
        assert_eq!((eps.len(), eps[0].hits, eps[0].attacks, eps[0].protected), (1, 50, 30, true));
        let (f, _) = evaluate_findings(&eps, &HashMap::new());
        assert!(f.iter().any(|x| x.rule_id == "oc-api-001"));
        let groups = group_counts(st, &org, "@appsec.category:attack_attempt", &["@http.client_ip".to_string()], range).await.unwrap();
        assert_eq!(groups, vec![(vec!["203.0.113.7".to_string()], 30.0)]);
    }
}
