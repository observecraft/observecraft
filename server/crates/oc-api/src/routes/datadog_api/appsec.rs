//! App & API Protection's In-App WAF configuration, Datadog-compatible
//! (`/api/v2/remote_config/products/asm/waf/{custom_rules,exclusion_filters,policies}`): the Terraform
//! provider's `datadog_appsec_waf_custom_rule` and `datadog_appsec_waf_exclusion_filter`, and the API
//! clients' WAF policies, on Postgres `appsec_waf_objects` (the UI's Policies pages read the same rows).
//!
//! - Documents keep the attributes as sent; the server adds `metadata` (custom rules, exclusion
//!   filters) or `meta` (policies) with who added and changed them, `options: {}` on conditions, and
//!   the exclusion filters' `search_query` (the security traces they exclude, in the search syntax).
//! - The managed policies (`recommended`, …) are listed with the org's own; they can't be changed.
//! - Errors use these endpoints' own shape: `{"errors": [{"title": …, "code": "400"}]}`.
//! - The configuration is stored and shown. Tracers receive WAF configuration through Remote
//!   Configuration, which ObserveCraft doesn't serve yet.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};

use super::users::require;
use super::DdAuth;
use crate::auth::{Access, AuthContext};
use crate::state::AppState;

type WafResult = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

fn err(status: StatusCode, title: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "errors": [{ "title": title, "code": status.as_u16().to_string() }] })))
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "appsec WAF configuration request failed");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// The kinds and their document type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    CustomRule,
    ExclusionFilter,
    Policy,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::CustomRule => "custom_rule",
            Kind::ExclusionFilter => "exclusion_filter",
            Kind::Policy => "policy",
        }
    }
}

/// The managed WAF policies: (id, name, description, default).
pub const MANAGED_POLICIES: [(&str, &str, &str, bool); 3] = [
    (
        "recommended",
        "Managed - Monitoring-only",
        "Monitor security scanners and application attacks such as Server-Side-Request-Forgery (SSRF), SQL Injection, Log4Shell, and Cross-Site-Scripting (XSS).",
        true,
    ),
    ("recommended-attack-tools", "Managed - Block attack tools", "Block known attack tools without impacting legitimate security scans. Monitor application attacks such as SSRF, SQL Injection, Log4Shell, and XSS.", false),
    (
        "recommended-blocking",
        "Managed - Block attack tools & application attacks",
        "Block known attack tools without impacting legitimate security scans. Block application attacks such as SSRF, SQL Injection, Log4Shell, and XSS. Rules are curated to reduce the risk of blocking legitimate traffic.",
        false,
    ),
];

/// A regular expression's syntax, enough to refuse the broken ones (a trailing escape, unbalanced
/// groups or classes).
pub fn regex_ok(re: &str) -> bool {
    let (mut depth, mut class, mut escaped) = (0i32, false, false);
    for c in re.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' if !class => class = true,
            ']' if class => class = false,
            '(' if !class => depth += 1,
            ')' if !class => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    !escaped && !class && depth == 0
}

/// Checks a document's attributes (Datadog's 400 messages).
pub fn check(kind: Kind, a: &Map<String, Value>) -> Result<(), &'static str> {
    match kind {
        Kind::CustomRule => {
            if a.get("name").and_then(Value::as_str).is_none_or(|n| n.trim().is_empty()) {
                return Err("failed to decode request");
            }
            let conditions = a.get("conditions").and_then(Value::as_array).ok_or("failed to decode request")?;
            for c in conditions {
                if let Some(re) = c.pointer("/parameters/regex").and_then(Value::as_str) {
                    if !regex_ok(re) {
                        return Err("failed to decode request");
                    }
                }
            }
        }
        Kind::ExclusionFilter => {
            if a.contains_key("event_query") {
                return Err("legacy exclusion filters cannot be created anymore");
            }
            if a.get("on_match").and_then(Value::as_str) == Some("monitor") && ["parameters", "path_glob", "rules_target", "scope"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_null())) {
                return Err("only IPs are supported for monitored exclusion filters");
            }
        }
        Kind::Policy => {
            if a.get("name").and_then(Value::as_str).is_none_or(|n| n.trim().is_empty()) {
                return Err("a policy needs a name");
            }
        }
    }
    Ok(())
}

fn quote(v: &str) -> String {
    if v.chars().any(|c| c.is_whitespace() || "-:()\"".contains(c)) {
        format!("\"{v}\"")
    } else {
        v.to_string()
    }
}

/// The security traces an exclusion filter excludes, in the search syntax.
pub fn search_query(a: &Map<String, Value>) -> String {
    let mut parts = Vec::new();
    let scopes: Vec<String> = a
        .get("scope")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|s| {
            let mut t = Vec::new();
            if let Some(e) = s.get("env").and_then(Value::as_str) {
                t.push(format!("env:{}", quote(e)));
            }
            if let Some(x) = s.get("service").and_then(Value::as_str) {
                t.push(format!("service:{}", quote(x)));
            }
            format!("({})", t.join(" AND "))
        })
        .collect();
    if !scopes.is_empty() {
        parts.push(scopes.join(" OR "));
    }
    if let Some(ips) = a.get("ip_list").and_then(Value::as_array).filter(|l| !l.is_empty()) {
        parts.push(ips.iter().filter_map(Value::as_str).map(|ip| format!("@http.client_ip:{ip}")).collect::<Vec<_>>().join(" OR "));
    }
    if let Some(p) = a.get("path_glob").and_then(Value::as_str).filter(|p| *p != "*") {
        parts.push(format!("@http.route:{}", quote(p)));
    }
    if let Some(params) = a.get("parameters").and_then(Value::as_array).filter(|l| !l.is_empty()) {
        parts.push(format!("({})", params.iter().filter_map(Value::as_str).map(|p| format!("@appsec.triggers.rule_matches.parameters.key:{}", quote(p))).collect::<Vec<_>>().join(" OR ")));
    }
    let targets: Vec<String> = a
        .get("rules_target")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| {
            let mut x = Vec::new();
            if let Some(id) = t.get("rule_id").and_then(Value::as_str) {
                x.push(format!("@appsec.rule_id:{id}"));
            }
            for (k, v) in t.get("tags").and_then(Value::as_object).into_iter().flatten() {
                if let Some(v) = v.as_str() {
                    x.push(format!("@appsec.{k}:{}", quote(v)));
                }
            }
            format!("({})", x.join(" AND "))
        })
        .collect();
    if !targets.is_empty() {
        parts.push(targets.join(" OR "));
    }
    parts.into_iter().map(|p| if p.contains(" OR ") && !p.starts_with('(') { format!("({p})") } else { p }).collect::<Vec<_>>().join(" AND ")
}

/// Attributes as stored: as sent, with `options: {}` on conditions and the server's fields left out.
fn normalize(kind: Kind, mut a: Map<String, Value>) -> Map<String, Value> {
    for k in ["metadata", "search_query", "basedOn"] {
        a.remove(k);
    }
    if kind == Kind::CustomRule {
        if let Some(Value::Array(conds)) = a.get_mut("conditions") {
            for c in conds {
                if let Some(p) = c.get_mut("parameters").and_then(Value::as_object_mut) {
                    p.entry("options").or_insert(json!({}));
                }
            }
        }
    }
    if kind == Kind::Policy {
        a.entry("isDefault").or_insert(json!(false));
        for k in ["rules", "rulesets", "scope"] {
            a.entry(k).or_insert(json!([]));
        }
        a.entry("version").or_insert(json!(-1));
    }
    a
}

#[derive(sqlx::FromRow)]
pub struct WafRow {
    pub id: String,
    pub attributes: Value,
    pub added_by: String,
    pub added_by_name: String,
    /// Epoch ms.
    pub added_at: i64,
    pub modified_by: Option<String>,
    pub modified_by_name: Option<String>,
    pub modified_at: Option<i64>,
}

const SELECT: &str = "SELECT id::text AS id, attributes, added_by, added_by_name, (extract(epoch FROM added_at) * 1000)::bigint AS added_at, modified_by, modified_by_name, \
    (extract(epoch FROM modified_at) * 1000)::bigint AS modified_at FROM appsec_waf_objects";

fn ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default()
}

pub fn doc(kind: Kind, r: &WafRow) -> Value {
    let mut meta = json!({ "added_at": ts(r.added_at), "added_by": r.added_by, "added_by_name": r.added_by_name });
    if let (Some(by), Some(at)) = (&r.modified_by, r.modified_at) {
        meta["modified_at"] = json!(ts(at));
        meta["modified_by"] = json!(by);
        meta["modified_by_name"] = json!(r.modified_by_name.clone().unwrap_or_default());
    }
    let mut attrs = r.attributes.as_object().cloned().unwrap_or_default();
    match kind {
        Kind::Policy => json!({ "id": r.id, "type": "policy", "attributes": attrs, "meta": meta }),
        _ => {
            if kind == Kind::ExclusionFilter {
                let q = search_query(&attrs);
                attrs.insert("search_query".into(), json!(q));
            }
            attrs.insert("metadata".into(), meta);
            json!({ "id": r.id, "type": kind.as_str(), "attributes": attrs })
        }
    }
}

fn managed_doc((id, name, description, default): (&str, &str, &str, bool)) -> Value {
    json!({ "id": id, "type": "policy", "attributes": { "description": description, "isDefault": default, "name": name, "rules": [], "rulesets": [], "scope": [], "version": 0 }, "meta": {} })
}

pub async fn list_rows(st: &AppState, org: &str, kind: Kind) -> Result<Vec<WafRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND kind = $2 ORDER BY added_at, id")).bind(org).bind(kind.as_str()).fetch_all(&st.postgres).await
}

async fn find(st: &AppState, org: &str, kind: Kind, id: &str) -> Result<Option<WafRow>, sqlx::Error> {
    if super::users::uuid(id).is_none() {
        return Ok(None);
    }
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND kind = $2 AND id = $3::uuid")).bind(org).bind(kind.as_str()).bind(id).fetch_optional(&st.postgres).await
}

/// The attributes of a request body, checked.
fn attributes(kind: Kind, raw: &Bytes) -> Result<Map<String, Value>, (StatusCode, Json<Value>)> {
    let v: Value = serde_json::from_slice(raw).map_err(|_| err(StatusCode::BAD_REQUEST, "failed to decode request"))?;
    let d = v.get("data").ok_or_else(|| err(StatusCode::BAD_REQUEST, "failed to decode request"))?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != kind.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "failed to decode request"));
    }
    let a = d.get("attributes").and_then(Value::as_object).cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "failed to decode request"))?;
    check(kind, &a).map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
    Ok(normalize(kind, a))
}

pub async fn insert(st: &AppState, ctx: &AuthContext, kind: Kind, attrs: &Map<String, Value>) -> Result<WafRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "WITH r AS (INSERT INTO appsec_waf_objects (org_id, kind, attributes, added_by, added_by_name) VALUES ($1, $2, $3, $4, $5) RETURNING *) \
         {} FROM r",
        SELECT.trim_end_matches(" FROM appsec_waf_objects")
    ))
    .bind(&ctx.org_id)
    .bind(kind.as_str())
    .bind(Value::Object(attrs.clone()))
    .bind(&ctx.user_id)
    .bind(&ctx.name)
    .fetch_one(&st.postgres)
    .await
}

pub async fn replace(st: &AppState, ctx: &AuthContext, kind: Kind, id: &str, attrs: &Map<String, Value>) -> Result<Option<WafRow>, sqlx::Error> {
    if super::users::uuid(id).is_none() {
        return Ok(None);
    }
    sqlx::query_as(&format!(
        "WITH r AS (UPDATE appsec_waf_objects SET attributes = $4, modified_by = $5, modified_by_name = $6, modified_at = now() \
         WHERE org_id = $1 AND kind = $2 AND id = $3::uuid RETURNING *) {} FROM r",
        SELECT.trim_end_matches(" FROM appsec_waf_objects")
    ))
    .bind(&ctx.org_id)
    .bind(kind.as_str())
    .bind(id)
    .bind(Value::Object(attrs.clone()))
    .bind(&ctx.user_id)
    .bind(&ctx.name)
    .fetch_optional(&st.postgres)
    .await
}

pub async fn remove(st: &AppState, org: &str, kind: Kind, id: &str) -> Result<bool, sqlx::Error> {
    if super::users::uuid(id).is_none() {
        return Ok(false);
    }
    Ok(sqlx::query("DELETE FROM appsec_waf_objects WHERE org_id = $1 AND kind = $2 AND id = $3::uuid").bind(org).bind(kind.as_str()).bind(id).execute(&st.postgres).await?.rows_affected() > 0)
}

async fn list(st: AppState, ctx: AuthContext, kind: Kind) -> WafResult {
    let rows = list_rows(&st, &ctx.org_id, kind).await.map_err(db_err)?;
    let mut docs: Vec<Value> = if kind == Kind::Policy { MANAGED_POLICIES.iter().map(|p| managed_doc(*p)).collect() } else { vec![] };
    docs.extend(rows.iter().map(|r| doc(kind, r)));
    Ok((StatusCode::OK, Json(json!({ "data": docs }))))
}

async fn get(st: AppState, ctx: AuthContext, kind: Kind, id: String) -> WafResult {
    if kind == Kind::Policy {
        if let Some(p) = MANAGED_POLICIES.iter().find(|p| p.0 == id) {
            return Ok((StatusCode::OK, Json(json!({ "data": managed_doc(*p) }))));
        }
    }
    let row = find(&st, &ctx.org_id, kind, &id).await.map_err(db_err)?.ok_or_else(|| err(StatusCode::NOT_FOUND, "id not found"))?;
    Ok((StatusCode::OK, Json(json!({ "data": doc(kind, &row) }))))
}

async fn create(st: AppState, ctx: AuthContext, kind: Kind, raw: Bytes) -> WafResult {
    require(&ctx, Access::Standard).map_err(|_| err(StatusCode::FORBIDDEN, "Forbidden"))?;
    let attrs = attributes(kind, &raw)?;
    let row = insert(&st, &ctx, kind, &attrs).await.map_err(db_err)?;
    // Datadog answers 200 for exclusion filters and 201 for the others.
    let status = if kind == Kind::ExclusionFilter { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(json!({ "data": doc(kind, &row) }))))
}

async fn update(st: AppState, ctx: AuthContext, kind: Kind, id: String, raw: Bytes) -> WafResult {
    require(&ctx, Access::Standard).map_err(|_| err(StatusCode::FORBIDDEN, "Forbidden"))?;
    if kind == Kind::Policy && MANAGED_POLICIES.iter().any(|p| p.0 == id) {
        return Err(err(StatusCode::BAD_REQUEST, "managed policies can't be changed"));
    }
    let attrs = attributes(kind, &raw)?;
    let row = replace(&st, &ctx, kind, &id, &attrs).await.map_err(db_err)?.ok_or_else(|| err(StatusCode::NOT_FOUND, "id not found"))?;
    Ok((StatusCode::OK, Json(json!({ "data": doc(kind, &row) }))))
}

async fn delete(st: AppState, ctx: AuthContext, kind: Kind, id: String) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    require(&ctx, Access::Standard).map_err(|_| err(StatusCode::FORBIDDEN, "Forbidden"))?;
    if remove(&st, &ctx.org_id, kind, &id).await.map_err(db_err)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(err(StatusCode::NOT_FOUND, "id not found"))
    }
}

macro_rules! waf_routes {
    ($kind:expr, $path:literal, $list:ident, $get:ident, $create:ident, $update:ident, $delete:ident, $op:literal) => {
        #[utoipa::path(get, path = $path, operation_id = concat!("datadog_list_waf_", $op), responses((status = 200, body = Object)), tag = "datadog-api")]
        pub async fn $list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> WafResult {
            list(st, ctx, $kind).await
        }
        #[utoipa::path(get, path = concat!($path, "/{id}"), operation_id = concat!("datadog_get_waf_", $op), params(("id" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
        pub async fn $get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> WafResult {
            get(st, ctx, $kind, id).await
        }
        #[utoipa::path(post, path = $path, operation_id = concat!("datadog_create_waf_", $op), request_body = Object, responses((status = 201, body = Object), (status = 400, body = Object)), tag = "datadog-api")]
        pub async fn $create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> WafResult {
            create(st, ctx, $kind, raw).await
        }
        #[utoipa::path(put, path = concat!($path, "/{id}"), operation_id = concat!("datadog_update_waf_", $op), params(("id" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
        pub async fn $update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> WafResult {
            update(st, ctx, $kind, id, raw).await
        }
        #[utoipa::path(delete, path = concat!($path, "/{id}"), operation_id = concat!("datadog_delete_waf_", $op), params(("id" = String, Path)), responses((status = 204), (status = 404, body = Object)), tag = "datadog-api")]
        pub async fn $delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, (StatusCode, Json<Value>)> {
            delete(st, ctx, $kind, id).await
        }
    };
}

waf_routes!(Kind::CustomRule, "/api/v2/remote_config/products/asm/waf/custom_rules", list_custom_rules, get_custom_rule, create_custom_rule, update_custom_rule, delete_custom_rule, "custom_rule");
waf_routes!(Kind::ExclusionFilter, "/api/v2/remote_config/products/asm/waf/exclusion_filters", list_exclusion_filters, get_exclusion_filter, create_exclusion_filter, update_exclusion_filter, delete_exclusion_filter, "exclusion_filter");
waf_routes!(Kind::Policy, "/api/v2/remote_config/products/asm/waf/policies", list_policies, get_policy, create_policy, update_policy, delete_policy, "policy");

#[cfg(test)]
mod tests {
    use super::super::replay::{Feature, TestServer};
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn checks_follow_datadogs_messages() {
        assert!(regex_ok("badactor") && regex_ok("a(b|c)[x)]\\d") && !regex_ok("\\") && !regex_ok("(a"));
        let rule = |re: &str| obj(json!({"name": "t", "conditions": [{"operator": "match_regex", "parameters": {"regex": re}}]}));
        assert!(check(Kind::CustomRule, &rule("badactor")).is_ok());
        assert_eq!(check(Kind::CustomRule, &rule("\\")), Err("failed to decode request"));
        assert!(check(Kind::ExclusionFilter, &obj(json!({"event_query": "test:1"}))).is_err());
        assert!(check(Kind::ExclusionFilter, &obj(json!({"on_match": "monitor", "ip_list": ["198.51.100.72"]}))).is_ok());
        assert!(check(Kind::ExclusionFilter, &obj(json!({"on_match": "monitor", "ip_list": ["198.51.100.72"], "path_glob": "/x"}))).is_err());
    }

    #[test]
    fn exclusion_filters_describe_their_traces() {
        let q = search_query(&obj(json!({"scope": [{"env": "staging", "service": "event-query"}], "path_glob": "*", "rules_target": [{"tags": {"category": "attack_attempt", "type": "xss"}}]})));
        assert_eq!(q, "(env:staging AND service:\"event-query\") AND (@appsec.category:attack_attempt AND @appsec.type:xss)");
        assert_eq!(search_query(&obj(json!({"ip_list": ["198.51.100.72"]}))), "@http.client_ip:198.51.100.72");
    }

    #[tokio::test]
    async fn replays_datadogs_application_security_scenarios() {
        let Some(f) = Feature::load("v2/application_security") else { return };
        let Some(s) = TestServer::start("dd-appsec-replay").await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= 10, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
