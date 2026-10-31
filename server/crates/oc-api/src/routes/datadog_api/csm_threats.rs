//! Workload Protection's configuration, Datadog-compatible: agent policies and agent rules
//! (`/api/v2/remote_config/products/cws/{policy,agent_rules}`, the Terraform provider's
//! `datadog_csm_threats_policy` and `datadog_csm_threats_agent_rule`), the policy download, and the
//! Agents and coverage analysis of the `csm/onboarding` endpoints, built from the Agents' host
//! metadata and inventory (`datadog_hosts`).
//!
//! - Policies and rules live in Postgres `cws_policies` / `cws_agent_rules`; the default ones
//!   (`workload_catalog.rs`) are rows too, installed per org by [`ensure_defaults`], and can be
//!   switched on and off but not changed or deleted.
//! - Errors use these endpoints' two shapes: `{"errors": ["…"]}` for validation and conflicts,
//!   `{"errors": [{"title": "…"}]}` for unknown ids.
//! - Agents receive policies through Remote Configuration, which ObserveCraft doesn't serve: the
//!   download (`/policy/download`) is a zip of policy files an Agent loads from
//!   `runtime_security_config.policies.dir`.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::users::require;
use super::DdAuth;
use crate::auth::{Access, AuthContext};
use crate::routes::workload_catalog::{self as catalog, AGENT_RULES, POLICIES};
use crate::state::AppState;

type CwsResult = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

/// `{"errors": ["…"]}`: validation errors and conflicts.
fn message(status: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "errors": [msg] })))
}

/// `{"errors": [{"title": "…"}]}`: unknown ids.
fn titled(status: StatusCode, title: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "errors": [{ "title": title }] })))
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "workload protection configuration request failed");
    message(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn forbidden(ctx: &AuthContext) -> Result<(), (StatusCode, Json<Value>)> {
    require(ctx, Access::Standard).map_err(|_| message(StatusCode::FORBIDDEN, "Forbidden"))
}

// ================================================================ defaults

fn installed() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Installs the default policies and agent rules of `org` once per process (existing rows, and
/// what users switched on or off, are kept).
pub async fn ensure_defaults(st: &AppState, org: &str) -> Result<(), sqlx::Error> {
    if installed().lock().unwrap().contains(org) {
        return Ok(());
    }
    for (key, name, description) in POLICIES {
        sqlx::query(
            "INSERT INTO cws_policies (org_id, id, name, description, enabled, is_default, pinned, updated_by_name) VALUES ($1, $2, $3, $4, true, true, true, 'ObserveCraft') \
             ON CONFLICT DO NOTHING",
        )
        .bind(org)
        .bind(catalog::policy_id(key))
        .bind(name)
        .bind(description)
        .execute(&st.postgres)
        .await?;
    }
    for r in AGENT_RULES {
        let mut tags = catalog::mitre_tags(r);
        if !r.pack.is_empty() {
            tags.push(format!("content_pack:{}", r.pack));
        }
        sqlx::query(
            "INSERT INTO cws_agent_rules (org_id, id, name, policy_id, description, expression, enabled, product_tags, is_default, created_by_name, updated_by_name) \
             VALUES ($1, $2, $3, $4, $5, $6, true, $7, true, 'ObserveCraft', 'ObserveCraft') ON CONFLICT DO NOTHING",
        )
        .bind(org)
        .bind(catalog::stable_id(&format!("rule:{}", r.name)))
        .bind(r.name)
        .bind(catalog::policy_id(r.policy))
        .bind(r.description)
        .bind(r.expression)
        .bind(&tags)
        .execute(&st.postgres)
        .await?;
    }
    installed().lock().unwrap().insert(org.to_string());
    Ok(())
}

// ================================================================ rows

#[derive(sqlx::FromRow, Clone)]
pub struct PolicyRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub host_tags_lists: Value,
    pub is_default: bool,
    pub pinned: bool,
    pub version: i32,
    pub updated_by_name: String,
    pub updated_by_handle: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(sqlx::FromRow, Clone)]
pub struct RuleRow {
    pub id: String,
    pub name: String,
    pub policy_id: String,
    pub description: String,
    pub expression: String,
    pub enabled: bool,
    pub silent: Option<bool>,
    pub blocking: bool,
    pub product_tags: Vec<String>,
    pub actions: Value,
    pub agent_version: String,
    pub is_default: bool,
    pub created_by_name: String,
    pub created_by_handle: String,
    pub updated_by_name: String,
    pub updated_by_handle: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const POLICY_SELECT: &str = "SELECT id, name, description, enabled, host_tags_lists, is_default, pinned, version, updated_by_name, updated_by_handle, \
    (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at FROM cws_policies";
const RULE_SELECT: &str = "SELECT id, name, policy_id, description, expression, enabled, silent, blocking, product_tags, actions, agent_version, is_default, \
    created_by_name, created_by_handle, updated_by_name, updated_by_handle, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at FROM cws_agent_rules";

pub async fn policies(st: &AppState, org: &str) -> Result<Vec<PolicyRow>, sqlx::Error> {
    ensure_defaults(st, org).await?;
    sqlx::query_as(&format!("{POLICY_SELECT} WHERE org_id = $1 ORDER BY is_default DESC, created_at, name")).bind(org).fetch_all(&st.postgres).await
}

pub async fn policy(st: &AppState, org: &str, id: &str) -> Result<Option<PolicyRow>, sqlx::Error> {
    ensure_defaults(st, org).await?;
    sqlx::query_as(&format!("{POLICY_SELECT} WHERE org_id = $1 AND id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await
}

pub async fn rules(st: &AppState, org: &str, policy_id: Option<&str>) -> Result<Vec<RuleRow>, sqlx::Error> {
    ensure_defaults(st, org).await?;
    sqlx::query_as(&format!("{RULE_SELECT} WHERE org_id = $1 AND ($2::text IS NULL OR policy_id = $2) ORDER BY is_default DESC, name"))
        .bind(org)
        .bind(policy_id)
        .fetch_all(&st.postgres)
        .await
}

pub async fn rule(st: &AppState, org: &str, id: &str) -> Result<Option<RuleRow>, sqlx::Error> {
    ensure_defaults(st, org).await?;
    sqlx::query_as(&format!("{RULE_SELECT} WHERE org_id = $1 AND id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await
}

/// (monitoring, blocking, disabled) rule counts per policy.
pub async fn rule_counts(st: &AppState, org: &str) -> Result<HashMap<String, (i64, i64, i64)>, sqlx::Error> {
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT policy_id, count(*) FILTER (WHERE enabled AND NOT blocking), count(*) FILTER (WHERE enabled AND blocking), count(*) FILTER (WHERE NOT enabled) \
         FROM cws_agent_rules WHERE org_id = $1 GROUP BY policy_id",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().map(|(p, m, b, d)| (p, (m, b, d))).collect())
}

pub fn policy_doc(p: &PolicyRow, counts: (i64, i64, i64)) -> Value {
    let (monitoring, blocking, disabled) = counts;
    json!({
        "id": p.id,
        "type": "policy",
        "attributes": {
            "blockingRulesCount": blocking,
            "datadogManaged": p.is_default,
            "description": p.description,
            "disabledRulesCount": disabled,
            "enabled": p.enabled,
            "hostTagsLists": p.host_tags_lists,
            "monitoringRulesCount": monitoring,
            "name": p.name,
            "pinned": p.pinned,
            "policyType": "policy",
            "policyVersion": p.version.to_string(),
            "ruleCount": monitoring + blocking + disabled,
            "updateDate": p.updated_at,
            "updater": { "name": p.updated_by_name, "handle": p.updated_by_handle },
        }
    })
}

pub fn rule_doc(r: &RuleRow) -> Value {
    let actions: Vec<Value> = r
        .actions
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut a| {
            if let Some(o) = a.as_object_mut() {
                o.entry("disabled").or_insert(json!(false));
            }
            a
        })
        .collect();
    let (monitoring, blocking, disabled): (Vec<&str>, Vec<&str>, Vec<&str>) = match (r.enabled, r.blocking, r.policy_id.is_empty()) {
        (_, _, true) => (vec![], vec![], vec![]),
        (true, false, _) => (vec![&r.policy_id], vec![], vec![]),
        (true, true, _) => (vec![], vec![&r.policy_id], vec![]),
        (false, _, _) => (vec![], vec![], vec![&r.policy_id]),
    };
    let mut a = json!({
        "actions": actions,
        "agentConstraint": r.agent_version,
        "blocking": blocking,
        "category": catalog::category(&r.expression),
        "creationDate": r.created_at,
        "creator": { "name": r.created_by_name, "handle": r.created_by_handle },
        "defaultRule": r.is_default,
        "description": r.description,
        "disabled": disabled,
        "enabled": r.enabled,
        "expression": r.expression,
        "filters": ["os == \"linux\""],
        "monitoring": monitoring,
        "name": r.name,
        "product_tags": r.product_tags,
        "updateDate": r.updated_at,
        "updater": { "name": r.updated_by_name, "handle": r.updated_by_handle },
    });
    if let Some(s) = r.silent {
        a["silent"] = json!(s);
    }
    json!({ "id": r.id, "type": "agent_rule", "attributes": a })
}

// ================================================================ checks

/// Whether a SECL expression is a boolean expression (enough to refuse a bare field such as
/// `exec.file.name`; the Agent compiles the rest).
pub fn expression_ok(expr: &str) -> bool {
    let e = expr.trim();
    if e.is_empty() {
        return false;
    }
    let (mut depth, mut quoted) = (0i32, false);
    for c in e.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' | '[' if !quoted => depth += 1,
            ')' | ']' if !quoted => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    if quoted || depth != 0 {
        return false;
    }
    ["==", "!=", "=~", "!~", " in ", " not in ", " allin ", ">", "<", "&&", "||"].iter().any(|op| e.contains(op))
}

fn rule_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A request's `data`, with its type checked.
fn data(raw: &Bytes, kind: &str) -> Result<Map<String, Value>, (StatusCode, Json<Value>)> {
    let v: Value = serde_json::from_slice(raw).map_err(|_| message(StatusCode::BAD_REQUEST, "input_validation_error(invalid JSON body)"))?;
    let d = v.get("data").and_then(Value::as_object).cloned().ok_or_else(|| message(StatusCode::BAD_REQUEST, "input_validation_error(missing data)"))?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != kind) {
        return Err(message(StatusCode::BAD_REQUEST, &format!("input_validation_error(type must be {kind})")));
    }
    Ok(d)
}

fn attrs(d: &Map<String, Value>) -> Map<String, Value> {
    d.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// `hostTagsLists`, or the legacy `hostTags` as a single list; both at once are refused.
fn host_tags(a: &Map<String, Value>) -> Result<Option<Value>, (StatusCode, Json<Value>)> {
    match (a.get("hostTags"), a.get("hostTagsLists")) {
        (Some(_), Some(_)) => Err(message(StatusCode::BAD_REQUEST, "input_validation_error(Field 'tags' is invalid: set hostTagsLists or the legacy hostTags, not both)")),
        (Some(t), None) => {
            let tags = str_list(Some(t));
            Ok(Some(if tags.is_empty() { json!([]) } else { json!([tags]) }))
        }
        (None, Some(l)) => {
            let lists: Vec<Vec<String>> = l.as_array().map(|a| a.iter().map(|x| str_list(Some(x))).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            Ok(Some(json!(lists)))
        }
        (None, None) => Ok(None),
    }
}

async fn handle(st: &AppState, ctx: &AuthContext) -> String {
    sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id::text = $1 AND org_id = $2").bind(&ctx.user_id).bind(&ctx.org_id).fetch_optional(&st.postgres).await.ok().flatten().unwrap_or_default()
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505")
}

// ================================================================ policies

pub async fn create_policy_row(st: &AppState, ctx: &AuthContext, a: &Map<String, Value>) -> Result<PolicyRow, (StatusCode, Json<Value>)> {
    let name = a.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(Field 'name' is invalid: a name is required)"));
    }
    let tags = host_tags(a)?.unwrap_or_else(|| json!([]));
    ensure_defaults(st, &ctx.org_id).await.map_err(db_err)?;
    let id = catalog::random_id();
    let by = handle(st, ctx).await;
    let r = sqlx::query(
        "INSERT INTO cws_policies (org_id, id, name, description, enabled, host_tags_lists, updated_by, updated_by_name, updated_by_handle) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(name)
    .bind(a.get("description").and_then(Value::as_str).unwrap_or(""))
    .bind(a.get("enabled").and_then(Value::as_bool).unwrap_or(false))
    .bind(&tags)
    .bind(&ctx.user_id)
    .bind(&ctx.name)
    .bind(&by)
    .execute(&st.postgres)
    .await;
    match r {
        Err(e) if is_unique_violation(&e) => return Err(message(StatusCode::CONFLICT, &format!("conflict(a policy named {name:?} already exists)"))),
        Err(e) => return Err(db_err(e)),
        Ok(_) => {}
    }
    policy(st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| message(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
}

pub async fn update_policy_row(st: &AppState, ctx: &AuthContext, id: &str, a: &Map<String, Value>) -> Result<PolicyRow, (StatusCode, Json<Value>)> {
    let current = policy(st, &ctx.org_id, id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "Not Found"))?;
    let tags = host_tags(a)?;
    if current.is_default && ((a.contains_key("name") && a.get("name").and_then(Value::as_str) != Some(current.name.as_str())) || tags.is_some()) {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(default policies can only be enabled or disabled)"));
    }
    let name = a.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&current.name).to_string();
    let by = handle(st, ctx).await;
    let r = sqlx::query(
        "UPDATE cws_policies SET name = $3, description = $4, enabled = $5, host_tags_lists = $6, version = version + 1, updated_by = $7, updated_by_name = $8, \
         updated_by_handle = $9, updated_at = now() WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(&name)
    .bind(a.get("description").and_then(Value::as_str).unwrap_or(&current.description))
    .bind(a.get("enabled").and_then(Value::as_bool).unwrap_or(current.enabled))
    .bind(tags.unwrap_or(current.host_tags_lists))
    .bind(&ctx.user_id)
    .bind(&ctx.name)
    .bind(&by)
    .execute(&st.postgres)
    .await;
    match r {
        Err(e) if is_unique_violation(&e) => return Err(message(StatusCode::CONFLICT, &format!("conflict(a policy named {name:?} already exists)"))),
        Err(e) => return Err(db_err(e)),
        Ok(_) => {}
    }
    policy(st, &ctx.org_id, id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "Not Found"))
}

pub async fn delete_policy_row(st: &AppState, org: &str, id: &str) -> Result<(), (StatusCode, Json<Value>)> {
    let current = policy(st, org, id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "Not Found"))?;
    if current.is_default {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(default policies can't be deleted)"));
    }
    let mut tx = st.postgres.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM cws_agent_rules WHERE org_id = $1 AND policy_id = $2").bind(org).bind(id).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("DELETE FROM cws_policies WHERE org_id = $1 AND id = $2").bind(org).bind(id).execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

#[utoipa::path(get, path = "/api/v2/remote_config/products/cws/policy", operation_id = "datadog_list_cws_policies", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_policies(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> CwsResult {
    let rows = policies(&st, &ctx.org_id).await.map_err(db_err)?;
    let counts = rule_counts(&st, &ctx.org_id).await.map_err(db_err)?;
    let docs: Vec<Value> = rows.iter().map(|p| policy_doc(p, counts.get(&p.id).copied().unwrap_or_default())).collect();
    Ok((StatusCode::OK, Json(json!({ "data": docs }))))
}

#[utoipa::path(get, path = "/api/v2/remote_config/products/cws/policy/{policy_id}", operation_id = "datadog_get_cws_policy", params(("policy_id" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get_policy(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> CwsResult {
    let p = policy(&st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "Not Found"))?;
    let counts = rule_counts(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(json!({ "data": policy_doc(&p, counts.get(&p.id).copied().unwrap_or_default()) }))))
}

#[utoipa::path(post, path = "/api/v2/remote_config/products/cws/policy", operation_id = "datadog_create_cws_policy", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 409, body = Object)), tag = "datadog-api")]
pub async fn create_policy(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> CwsResult {
    forbidden(&ctx)?;
    let a = attrs(&data(&raw, "policy")?);
    let p = create_policy_row(&st, &ctx, &a).await?;
    Ok((StatusCode::OK, Json(json!({ "data": policy_doc(&p, (0, 0, 0)) }))))
}

#[utoipa::path(patch, path = "/api/v2/remote_config/products/cws/policy/{policy_id}", operation_id = "datadog_update_cws_policy", params(("policy_id" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update_policy(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> CwsResult {
    forbidden(&ctx)?;
    let d = data(&raw, "policy")?;
    if d.get("id").and_then(Value::as_str).is_some_and(|i| i != id) {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(the body's id doesn't match the path)"));
    }
    let p = update_policy_row(&st, &ctx, &id, &attrs(&d)).await?;
    let counts = rule_counts(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(json!({ "data": policy_doc(&p, counts.get(&p.id).copied().unwrap_or_default()) }))))
}

#[utoipa::path(delete, path = "/api/v2/remote_config/products/cws/policy/{policy_id}", operation_id = "datadog_delete_cws_policy", params(("policy_id" = String, Path)), responses((status = 204), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete_policy(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    forbidden(&ctx)?;
    delete_policy_row(&st, &ctx.org_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ agent rules

#[derive(Deserialize, Default)]
pub struct PolicyParam {
    pub policy_id: Option<String>,
}

fn actions(a: &Map<String, Value>) -> Result<Value, (StatusCode, Json<Value>)> {
    let list = a.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(list.len());
    for x in list {
        let o = x.as_object().cloned().unwrap_or_default();
        let mut kept = Map::new();
        if let Some(s) = o.get("set").and_then(Value::as_object) {
            if s.get("name").and_then(Value::as_str).is_none_or(str::is_empty) {
                return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(Field 'actions' is invalid: a set action needs a name)"));
            }
            kept.insert("set".into(), Value::Object(s.clone()));
        }
        if let Some(h) = o.get("hash").and_then(Value::as_object) {
            kept.insert("hash".into(), Value::Object(h.clone()));
        }
        for k in ["kill", "filter", "coredump"] {
            if let Some(v) = o.get(k) {
                kept.insert(k.into(), v.clone());
            }
        }
        if kept.is_empty() {
            return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(Field 'actions' is invalid: an action needs set or hash)"));
        }
        out.push(Value::Object(kept));
    }
    Ok(Value::Array(out))
}

fn expression_error(name: &str, expr: &str) -> (StatusCode, Json<Value>) {
    message(StatusCode::BAD_REQUEST, &format!("input_validation_error(Field 'expression' is invalid: rule `{name}`: a boolean expression is expected: {expr})"))
}

pub async fn create_rule_row(st: &AppState, ctx: &AuthContext, a: &Map<String, Value>) -> Result<RuleRow, (StatusCode, Json<Value>)> {
    let name = a.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !rule_name_ok(&name) {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(Field 'name' is invalid: lowercase letters, digits and underscores only)"));
    }
    let expression = a.get("expression").and_then(Value::as_str).unwrap_or("").to_string();
    if !expression_ok(&expression) {
        return Err(expression_error(&name, &expression));
    }
    let policy_id = a.get("policy_id").and_then(Value::as_str).unwrap_or("").to_string();
    if !policy_id.is_empty() && policy(st, &ctx.org_id, &policy_id).await.map_err(db_err)?.is_none() {
        return Err(titled(StatusCode::NOT_FOUND, "policy not found"));
    }
    let acts = actions(a)?;
    ensure_defaults(st, &ctx.org_id).await.map_err(db_err)?;
    let id = catalog::random_id();
    let by = handle(st, ctx).await;
    let r = sqlx::query(
        "INSERT INTO cws_agent_rules (org_id, id, name, policy_id, description, expression, enabled, silent, blocking, product_tags, actions, agent_version, \
         created_by_name, created_by_handle, updated_by_name, updated_by_handle) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $13, $14)",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&name)
    .bind(&policy_id)
    .bind(a.get("description").and_then(Value::as_str).unwrap_or(""))
    .bind(&expression)
    .bind(a.get("enabled").and_then(Value::as_bool).unwrap_or(true))
    .bind(a.get("silent").and_then(Value::as_bool))
    .bind(a.get("blocking").and_then(Value::as_array).is_some_and(|b| !b.is_empty()))
    .bind(str_list(a.get("product_tags")))
    .bind(&acts)
    .bind(a.get("agent_version").and_then(Value::as_str).unwrap_or(""))
    .bind(&ctx.name)
    .bind(&by)
    .execute(&st.postgres)
    .await;
    match r {
        Err(e) if is_unique_violation(&e) => return Err(message(StatusCode::CONFLICT, &format!("conflict(an agent rule named {name:?} already exists in this policy)"))),
        Err(e) => return Err(db_err(e)),
        Ok(_) => {}
    }
    rule(st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| message(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
}

pub async fn update_rule_row(st: &AppState, ctx: &AuthContext, id: &str, a: &Map<String, Value>) -> Result<RuleRow, (StatusCode, Json<Value>)> {
    let current = rule(st, &ctx.org_id, id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "agent rule not found"))?;
    let expression = a.get("expression").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| current.expression.clone());
    if current.is_default && expression != current.expression {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(default agent rules can only be enabled or disabled)"));
    }
    if !expression_ok(&expression) {
        return Err(expression_error(&current.name, &expression));
    }
    let acts = if a.contains_key("actions") { actions(a)? } else { current.actions.clone() };
    let by = handle(st, ctx).await;
    sqlx::query(
        "UPDATE cws_agent_rules SET description = $3, expression = $4, enabled = $5, silent = $6, product_tags = $7, actions = $8, agent_version = $9, \
         updated_by_name = $10, updated_by_handle = $11, updated_at = now() WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(a.get("description").and_then(Value::as_str).unwrap_or(&current.description))
    .bind(&expression)
    .bind(a.get("enabled").and_then(Value::as_bool).unwrap_or(current.enabled))
    .bind(a.get("silent").and_then(Value::as_bool).or(current.silent))
    .bind(if a.contains_key("product_tags") { str_list(a.get("product_tags")) } else { current.product_tags.clone() })
    .bind(&acts)
    .bind(a.get("agent_version").and_then(Value::as_str).unwrap_or(&current.agent_version))
    .bind(&ctx.name)
    .bind(&by)
    .execute(&st.postgres)
    .await
    .map_err(db_err)?;
    rule(st, &ctx.org_id, id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "agent rule not found"))
}

pub async fn delete_rule_row(st: &AppState, org: &str, id: &str) -> Result<(), (StatusCode, Json<Value>)> {
    let current = rule(st, org, id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "agent rule not found"))?;
    if current.is_default {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(default agent rules can't be deleted, disable them instead)"));
    }
    sqlx::query("DELETE FROM cws_agent_rules WHERE org_id = $1 AND id = $2").bind(org).bind(id).execute(&st.postgres).await.map_err(db_err)?;
    Ok(())
}

#[utoipa::path(get, path = "/api/v2/remote_config/products/cws/agent_rules", operation_id = "datadog_list_cws_agent_rules", params(("policy_id" = Option<String>, Query)), responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_rules(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<PolicyParam>) -> CwsResult {
    let rows = rules(&st, &ctx.org_id, q.policy_id.as_deref().filter(|p| !p.is_empty())).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(json!({ "data": rows.iter().map(rule_doc).collect::<Vec<_>>() }))))
}

#[utoipa::path(get, path = "/api/v2/remote_config/products/cws/agent_rules/{agent_rule_id}", operation_id = "datadog_get_cws_agent_rule", params(("agent_rule_id" = String, Path), ("policy_id" = Option<String>, Query)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> CwsResult {
    let r = rule(&st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(|| titled(StatusCode::NOT_FOUND, "agent rule not found"))?;
    Ok((StatusCode::OK, Json(json!({ "data": rule_doc(&r) }))))
}

#[utoipa::path(post, path = "/api/v2/remote_config/products/cws/agent_rules", operation_id = "datadog_create_cws_agent_rule", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 409, body = Object)), tag = "datadog-api")]
pub async fn create_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> CwsResult {
    forbidden(&ctx)?;
    let a = attrs(&data(&raw, "agent_rule")?);
    let r = create_rule_row(&st, &ctx, &a).await?;
    Ok((StatusCode::OK, Json(json!({ "data": rule_doc(&r) }))))
}

#[utoipa::path(patch, path = "/api/v2/remote_config/products/cws/agent_rules/{agent_rule_id}", operation_id = "datadog_update_cws_agent_rule", params(("agent_rule_id" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> CwsResult {
    forbidden(&ctx)?;
    let d = data(&raw, "agent_rule")?;
    if d.get("id").and_then(Value::as_str).is_some_and(|i| i != id) {
        return Err(message(StatusCode::BAD_REQUEST, "input_validation_error(the body's id doesn't match the path)"));
    }
    let r = update_rule_row(&st, &ctx, &id, &attrs(&d)).await?;
    Ok((StatusCode::OK, Json(json!({ "data": rule_doc(&r) }))))
}

#[utoipa::path(delete, path = "/api/v2/remote_config/products/cws/agent_rules/{agent_rule_id}", operation_id = "datadog_delete_cws_agent_rule", params(("agent_rule_id" = String, Path), ("policy_id" = Option<String>, Query)), responses((status = 204), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    forbidden(&ctx)?;
    delete_rule_row(&st, &ctx.org_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ download

/// A policy file, in the format the Agent loads from `runtime_security_config.policies.dir`.
pub fn policy_file(p: &PolicyRow, rules: &[RuleRow]) -> String {
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut out = format!("# {} ({}), from ObserveCraft.\nversion: {}\nrules:\n", p.name, p.id, q(&p.version.to_string()));
    for r in rules.iter().filter(|r| r.policy_id == p.id) {
        out.push_str(&format!("  - id: {}\n    version: {}\n    description: {}\n    expression: {}\n", r.name, q(&r.updated_at.to_string()), q(&r.description), q(&r.expression)));
        if !r.enabled {
            out.push_str("    disabled: true\n");
        }
        if r.silent == Some(true) {
            out.push_str("    silent: true\n");
        }
        if !r.agent_version.is_empty() {
            out.push_str(&format!("    agent_version: {}\n", q(&r.agent_version)));
        }
        if !r.product_tags.is_empty() {
            out.push_str("    tags:\n");
            for t in &r.product_tags {
                let (k, v) = t.split_once(':').unwrap_or((t, ""));
                out.push_str(&format!("      {}: {}\n", q(k), q(v)));
            }
        }
        if r.actions.as_array().is_some_and(|a| !a.is_empty()) {
            out.push_str(&format!("    actions: {}\n", r.actions));
        }
    }
    out
}

/// A zip archive of `files` (stored, not compressed).
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, body) in files {
        let mut crc = flate2::Crc::new();
        crc.update(body);
        let offset = out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.write_all(&sig.to_le_bytes()).unwrap();
            if central {
                h.write_all(&20u16.to_le_bytes()).unwrap();
            }
            for v in [20u16, 0, 0, 0, 0x21] {
                h.write_all(&v.to_le_bytes()).unwrap();
            }
            for v in [crc.sum(), body.len() as u32, body.len() as u32] {
                h.write_all(&v.to_le_bytes()).unwrap();
            }
            h.write_all(&(name.len() as u16).to_le_bytes()).unwrap();
            h.write_all(&0u16.to_le_bytes()).unwrap();
            if central {
                for v in [0u16, 0, 0] {
                    h.write_all(&v.to_le_bytes()).unwrap();
                }
                h.write_all(&0u32.to_le_bytes()).unwrap();
                h.write_all(&offset.to_le_bytes()).unwrap();
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend_from_slice(body);
        central.extend(header(0x0201_4b50, true));
    }
    let start = out.len() as u32;
    let size = central.len() as u32;
    out.extend(central);
    out.write_all(&0x0605_4b50u32.to_le_bytes()).unwrap();
    for v in [0u16, 0, files.len() as u16, files.len() as u16] {
        out.write_all(&v.to_le_bytes()).unwrap();
    }
    for v in [size, start] {
        out.write_all(&v.to_le_bytes()).unwrap();
    }
    out.write_all(&0u16.to_le_bytes()).unwrap();
    out
}

/// The org's enabled policies, one policy file each.
pub async fn policy_archive(st: &AppState, org: &str) -> Result<Vec<u8>, sqlx::Error> {
    let ps = policies(st, org).await?;
    let rs = rules(st, org, None).await?;
    let files: Vec<(String, Vec<u8>)> = ps
        .iter()
        .filter(|p| p.enabled)
        .map(|p| {
            let slug: String = p.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            (format!("{}_{}.policy", slug.trim_matches('_'), p.id), policy_file(p, &rs).into_bytes())
        })
        .collect();
    Ok(zip(&files))
}

#[utoipa::path(get, path = "/api/v2/remote_config/products/cws/policy/download", operation_id = "datadog_download_cws_policy", responses((status = 200, content_type = "application/zip", body = Vec<u8>)), tag = "datadog-api")]
pub async fn download(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> Response {
    match policy_archive(&st, &ctx.org_id).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/zip"), (header::CONTENT_DISPOSITION, "attachment; filename=\"policies.zip\"")], bytes).into_response(),
        Err(e) => db_err(e).into_response(),
    }
}

// ================================================================ agents and coverage

/// An Agent as the `csm/onboarding` endpoints describe it, from its host metadata and inventory.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CsmAgent {
    pub id: String,
    pub host_id: i64,
    pub hostname: String,
    pub agent_version: String,
    pub os: String,
    pub envs: Vec<String>,
    pub cluster_name: Vec<String>,
    /// ECS or EKS for Fargate tasks, empty otherwise.
    pub aws_fargate: String,
    pub ecs_fargate_task_arn: String,
    pub install_method_tool: String,
    pub install_method_installer_version: String,
    pub is_cws_enabled: bool,
    pub is_cspm_enabled: bool,
    pub is_csm_vm_hosts_enabled: bool,
    pub is_csm_vm_containers_enabled: bool,
    pub is_remote_configuration_enabled: bool,
    pub is_cws_remote_configuration_enabled: bool,
    /// The host's `key:value` tags.
    pub tags: Vec<String>,
    /// The OS's brand logo key (`web/src/assets/brandLogos.generated.ts`): linux, windows or darwin.
    pub os_logo: String,
    /// Where the Agent runs, as a brand logo key: amazonecs / amazoneks (Fargate), kubernetes, or "".
    pub platform_logo: String,
    /// Epoch ms of the last host metadata.
    pub last_seen: i64,
}

fn hash64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[derive(sqlx::FromRow)]
struct HostRow {
    host: String,
    agent_version: String,
    os: String,
    platform: String,
    host_tags: Vec<String>,
    payload: Value,
    agent_inventory: Option<Value>,
    updated_at: i64,
}

/// Every Agent of the org that reported host metadata in the last day.
pub async fn agents(st: &AppState, org: &str) -> Result<Vec<CsmAgent>, sqlx::Error> {
    let rows: Vec<HostRow> = sqlx::query_as(
        "SELECT host, agent_version, os, platform, host_tags, payload, agent_inventory, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at \
         FROM datadog_hosts WHERE org_id = $1 AND updated_at > now() - interval '1 day' ORDER BY host",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().map(|r| agent_of(org, r)).collect())
}

fn agent_of(org: &str, r: HostRow) -> CsmAgent {
    let inv = r.agent_inventory.unwrap_or(Value::Null);
    let flag = |k: &str| inv.get(k).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false);
    let text = |k: &str| inv.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let tag_values = |key: &str| -> Vec<String> {
        let mut v: Vec<String> = r.host_tags.iter().filter_map(|t| t.strip_prefix(key).and_then(|x| x.strip_prefix(':')).map(str::to_string)).collect();
        v.sort();
        v.dedup();
        v
    };
    let task_arn = text("ecs_fargate_task_arn");
    let aws_fargate = if !task_arn.is_empty() {
        "ECS".to_string()
    } else if flag("eks_fargate") {
        "EKS".to_string()
    } else {
        String::new()
    };
    let mut clusters = tag_values("kube_cluster_name");
    for k in ["ecs_fargate_cluster_name", "eks_fargate_cluster_name"] {
        let c = text(k);
        if !c.is_empty() && !clusters.contains(&c) {
            clusters.push(c);
        }
    }
    if clusters.is_empty() {
        clusters = tag_values("ecs_cluster_name");
    }
    let install = r.payload.get("install-method").cloned().unwrap_or(Value::Null);
    let tool = Some(text("install_method_tool")).filter(|s| !s.is_empty()).or_else(|| install.get("tool").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| "undefined".into());
    let installer = Some(text("install_method_installer_version")).filter(|s| !s.is_empty()).or_else(|| install.get("installer_version").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    let os = if r.os.eq_ignore_ascii_case("linux") || r.os.is_empty() && !r.platform.is_empty() { "GNU/Linux".to_string() } else { r.os.clone() };
    let h = hash64(&format!("{org}/{}", r.host));
    let envs = tag_values("env");
    let lower = format!("{} {}", r.os, r.platform).to_lowercase();
    let os_logo = if lower.contains("windows") { "windows" } else if lower.contains("darwin") || lower.contains("mac") { "darwin" } else { "linux" }.to_string();
    let platform_logo = match aws_fargate.as_str() {
        "ECS" => "amazonecs",
        "EKS" => "amazoneks",
        _ if !tag_values("kube_cluster_name").is_empty() => "kubernetes",
        _ => "",
    }
    .to_string();
    CsmAgent {
        id: format!("{:016x}{:016x}", h, hash64(&r.host)),
        host_id: (h >> 1) as i64,
        hostname: r.host,
        agent_version: r.agent_version,
        os,
        envs,
        cluster_name: clusters,
        aws_fargate,
        ecs_fargate_task_arn: task_arn,
        install_method_tool: tool,
        install_method_installer_version: installer,
        is_cws_enabled: flag("feature_cws_enabled"),
        is_cspm_enabled: flag("feature_cspm_enabled"),
        is_csm_vm_hosts_enabled: flag("feature_csm_vm_hosts_enabled"),
        is_csm_vm_containers_enabled: flag("feature_csm_vm_containers_enabled"),
        is_remote_configuration_enabled: flag("feature_remote_configuration_enabled"),
        is_cws_remote_configuration_enabled: flag("feature_cws_remote_config_enabled"),
        tags: r.host_tags,
        os_logo,
        platform_logo,
        last_seen: r.updated_at,
    }
}

fn agent_doc(a: &CsmAgent) -> Value {
    json!({
        "id": a.id,
        "type": "datadog_agent",
        "attributes": {
            "agent_version": a.agent_version,
            "aws_fargate": a.aws_fargate,
            "cluster_name": if a.cluster_name.is_empty() { vec![String::new()] } else { a.cluster_name.clone() },
            "ecs_fargate_task_arn": a.ecs_fargate_task_arn,
            "envs": a.envs,
            "host_id": a.host_id,
            "hostname": a.hostname,
            "install_method_installer_version": a.install_method_installer_version,
            "install_method_tool": a.install_method_tool,
            "is_csm_vm_containers_enabled": a.is_csm_vm_containers_enabled,
            "is_csm_vm_hosts_enabled": a.is_csm_vm_hosts_enabled,
            "is_cspm_enabled": a.is_cspm_enabled,
            "is_cws_enabled": a.is_cws_enabled,
            "is_cws_remote_configuration_enabled": a.is_cws_remote_configuration_enabled,
            "is_remote_configuration_enabled": a.is_remote_configuration_enabled,
            "os": a.os,
        }
    })
}

#[derive(Deserialize, Default)]
pub struct AgentsQuery {
    pub page: Option<usize>,
    pub size: Option<usize>,
    pub query: Option<String>,
    pub order_direction: Option<String>,
}

fn agents_page(all: Vec<CsmAgent>, q: &AgentsQuery) -> Value {
    let needle = q.query.as_deref().unwrap_or("").trim().to_lowercase();
    let mut matched: Vec<CsmAgent> = all
        .into_iter()
        .filter(|a| needle.is_empty() || a.hostname.to_lowercase().contains(&needle) || a.envs.iter().any(|e| e.to_lowercase().contains(&needle)) || a.agent_version.contains(&needle))
        .collect();
    if q.order_direction.as_deref() == Some("desc") {
        matched.reverse();
    }
    let size = q.size.unwrap_or(10).clamp(1, 100);
    let page = q.page.unwrap_or(0);
    let data: Vec<Value> = matched.iter().skip(page * size).take(size).map(agent_doc).collect();
    json!({ "data": data, "meta": { "total_filtered": matched.len(), "page_index": page, "page_size": size } })
}

#[utoipa::path(get, path = "/api/v2/csm/onboarding/agents", operation_id = "datadog_list_csm_agents", params(("page" = Option<usize>, Query), ("size" = Option<usize>, Query), ("query" = Option<String>, Query), ("order_direction" = Option<String>, Query)), responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_agents(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<AgentsQuery>) -> CwsResult {
    let all = agents(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(agents_page(all.into_iter().filter(|a| a.aws_fargate.is_empty()).collect(), &q))))
}

#[utoipa::path(get, path = "/api/v2/csm/onboarding/serverless/agents", operation_id = "datadog_list_csm_serverless_agents", params(("page" = Option<usize>, Query), ("size" = Option<usize>, Query), ("query" = Option<String>, Query), ("order_direction" = Option<String>, Query)), responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_serverless_agents(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<AgentsQuery>) -> CwsResult {
    let all = agents(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(agents_page(all.into_iter().filter(|a| !a.aws_fargate.is_empty()).collect(), &q))))
}

/// A coverage block: (configured, partially configured, total).
fn coverage(configured: usize, partial: usize, total: usize) -> Value {
    json!({
        "configured_resources_count": configured,
        "partially_configured_resources_count": partial,
        "total_resources_count": total,
        "coverage": if total == 0 { 0.0 } else { configured as f64 / total as f64 },
    })
}

/// The org's id as Datadog's numeric org id.
fn numeric_org(org: &str) -> i64 {
    (hash64(org) >> 33) as i64
}

fn coverage_doc(org: &str, kind: &str, attributes: Value) -> Value {
    let mut a = attributes;
    a["org_id"] = json!(numeric_org(org));
    json!({ "data": { "id": catalog::stable_id(&format!("coverage:{org}")), "type": format!("get_{kind}_coverage_analysis_response_public_v0"), "attributes": a } })
}

/// Coverage of hosts and containers: Workload Protection, Misconfigurations and Vulnerability Management.
pub fn hosts_coverage(agents: &[CsmAgent]) -> Value {
    let hosts: Vec<&CsmAgent> = agents.iter().filter(|a| a.aws_fargate.is_empty()).collect();
    let n = hosts.len();
    let cws = hosts.iter().filter(|a| a.is_cws_enabled).count();
    let cspm = hosts.iter().filter(|a| a.is_cspm_enabled).count();
    let vm = hosts.iter().filter(|a| a.is_csm_vm_hosts_enabled || a.is_csm_vm_containers_enabled).count();
    let all = hosts.iter().filter(|a| a.is_cws_enabled && a.is_cspm_enabled).count();
    let some = hosts.iter().filter(|a| (a.is_cws_enabled || a.is_cspm_enabled) && !(a.is_cws_enabled && a.is_cspm_enabled)).count();
    json!({ "cws_coverage": coverage(cws, 0, n), "cspm_coverage": coverage(cspm, 0, n), "vm_coverage": coverage(vm, 0, n), "total_coverage": coverage(all, some, n) })
}

pub fn serverless_coverage(agents: &[CsmAgent]) -> Value {
    let tasks: Vec<&CsmAgent> = agents.iter().filter(|a| !a.aws_fargate.is_empty()).collect();
    let cws = tasks.iter().filter(|a| a.is_cws_enabled).count();
    json!({ "cws_coverage": coverage(cws, 0, tasks.len()), "total_coverage": coverage(cws, 0, tasks.len()) })
}

#[utoipa::path(get, path = "/api/v2/csm/onboarding/coverage_analysis/hosts_and_containers", operation_id = "datadog_csm_hosts_coverage", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn coverage_hosts(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> CwsResult {
    let all = agents(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(coverage_doc(&ctx.org_id, "hosts_and_containers", hosts_coverage(&all)))))
}

#[utoipa::path(get, path = "/api/v2/csm/onboarding/coverage_analysis/serverless", operation_id = "datadog_csm_serverless_coverage", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn coverage_serverless(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> CwsResult {
    let all = agents(&st, &ctx.org_id).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(coverage_doc(&ctx.org_id, "serverless", serverless_coverage(&all)))))
}

/// Cloud accounts aren't scanned (Cloud Security's account scanning is still missing): no resources.
#[utoipa::path(get, path = "/api/v2/csm/onboarding/coverage_analysis/cloud_accounts", operation_id = "datadog_csm_cloud_accounts_coverage", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn coverage_cloud_accounts(DdAuth(ctx): DdAuth) -> CwsResult {
    let none = || coverage(0, 0, 0);
    Ok((StatusCode::OK, Json(coverage_doc(&ctx.org_id, "cloud_accounts", json!({ "aws_coverage": none(), "azure_coverage": none(), "gcp_coverage": none(), "total_coverage": none() })))))
}

#[cfg(test)]
mod tests {
    use super::super::replay::{Feature, TestServer};
    use super::*;

    #[test]
    fn expressions_must_be_boolean() {
        assert!(expression_ok(r#"exec.file.name == "sh""#));
        assert!(expression_ok(r#"open.file.path in ["/etc/shadow", "/etc/passwd"] && process.uid != 0"#));
        assert!(!expression_ok("exec.file.name"));
        assert!(!expression_ok(r#"exec.file.name == "sh"#));
        assert!(!expression_ok(r#"(exec.file.name == "sh""#));
        assert!(rule_name_ok("my_agent_rule_1") && !rule_name_ok("My Rule") && !rule_name_ok(""));
    }

    #[test]
    fn host_tags_take_one_form() {
        let a = |v: Value| v.as_object().cloned().unwrap();
        assert_eq!(host_tags(&a(json!({"hostTags": ["env:staging"]}))).unwrap(), Some(json!([["env:staging"]])));
        assert_eq!(host_tags(&a(json!({"hostTagsLists": [["env:prod", "team:a"], []]}))).unwrap(), Some(json!([["env:prod", "team:a"]])));
        assert!(host_tags(&a(json!({"hostTags": [], "hostTagsLists": []}))).is_err());
        assert_eq!(host_tags(&a(json!({}))).unwrap(), None);
    }

    #[test]
    fn the_download_is_a_zip_of_policy_files() {
        let z = zip(&[("a.policy".into(), b"version: 1\n".to_vec()), ("b.policy".into(), b"rules:\n".to_vec())]);
        assert_eq!(&z[..4], b"PK\x03\x04");
        assert_eq!(&z[z.len() - 22..z.len() - 18], b"PK\x05\x06");
        assert!(z.windows(8).any(|w| w == b"a.policy"));
    }

    #[test]
    fn agents_come_from_host_metadata() {
        let row = |host: &str, inv: Value, tags: &[&str]| HostRow {
            host: host.into(),
            agent_version: "7.70.0".into(),
            os: "linux".into(),
            platform: "ubuntu".into(),
            host_tags: tags.iter().map(|s| s.to_string()).collect(),
            payload: json!({"install-method": {"tool": "helm", "installer_version": "datadog-3.120.0"}}),
            agent_inventory: Some(inv),
            updated_at: 0,
        };
        let a = agent_of("o", row("i-0a1b2c3d4e5f60001", json!({"feature_cws_enabled": true}), &["env:prod", "kube_cluster_name:shop-prod"]));
        assert!(a.is_cws_enabled && !a.is_cspm_enabled && a.aws_fargate.is_empty());
        assert_eq!((a.os_logo.as_str(), a.platform_logo.as_str()), ("linux", "kubernetes"));
        assert_eq!((a.envs.as_slice(), a.cluster_name.as_slice(), a.install_method_tool.as_str(), a.os.as_str()), (&["prod".to_string()][..], &["shop-prod".to_string()][..], "helm", "GNU/Linux"));
        let t = agent_of("o", row("task-1", json!({"ecs_fargate_task_arn": "arn:aws:ecs:us-east-1:123456789012:task/shop/abc", "ecs_fargate_cluster_name": "shop"}), &[]));
        assert_eq!((t.aws_fargate.as_str(), t.cluster_name.as_slice(), t.platform_logo.as_str()), ("ECS", &["shop".to_string()][..], "amazonecs"));
        let cov = hosts_coverage(&[a, t]);
        assert_eq!(cov["cws_coverage"]["configured_resources_count"], 1);
        assert_eq!(cov["total_coverage"]["partially_configured_resources_count"], 1);
    }

    async fn replay(feature: &str, prefix: &str, min: usize) {
        let Some(f) = Feature::load(feature) else { return };
        let Some(s) = TestServer::start(prefix).await else { return };
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
        assert!(replayed >= min, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[tokio::test]
    async fn replays_datadogs_csm_threats_scenarios() {
        replay("v2/csm_threats", "dd-cws-replay", 15).await;
    }

    #[tokio::test]
    async fn replays_datadogs_csm_agents_and_coverage_scenarios() {
        replay("v2/csm_agents", "dd-csm-agents-replay", 2).await;
        replay("v2/csm_coverage_analysis", "dd-csm-coverage-replay", 3).await;
    }
}
