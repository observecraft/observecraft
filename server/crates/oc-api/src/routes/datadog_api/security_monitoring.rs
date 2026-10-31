//! Cloud SIEM, Datadog-compatible (`/api/v2/security_monitoring/...`): the Terraform provider's
//! `datadog_security_monitoring_rule` and `datadog_security_monitoring_suppression`, and the API
//! clients' signal reads and triage, on the records the UI uses ([`crate::routes::security_rules`],
//! [`crate::routes::security_suppressions`], [`crate::routes::security_signals`]).
//!
//! - Rules are plain camelCase documents (not JSON:API), as Datadog sends them. Fields we don't
//!   interpret (reference tables, `groupSignalsBy`, scheduling, …) are kept and sent back.
//!   An update (`PUT`) changes the fields it sends. Default rules (our content packs) are listed
//!   with `isDefault`; only `isEnabled` changes on them, and they can't be deleted.
//! - Signals are `signal` documents with Datadog's `custom` attributes (`title`, `status`,
//!   `workflow.rule`, `workflow.triage`); the triage answers are `signal_metadata` documents.
//! - Suppressions are JSON:API `suppressions` documents.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use base64::Engine;
use serde_json::{json, Map, Value};

use super::users::{bad_request, body, data, not_found, ok, require, DocResult};
use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::routes::logs::TimeRange;
use crate::routes::security_rules::{delete_rule_row, insert_rule, load_rule, load_rules, update_rule_row, SecurityRule, SecurityRuleInput};
use crate::routes::security_signals::{load_signal, matches, set_assignee, set_state, signals_in, SecuritySignal, SignalStateInput};
use crate::routes::security_suppressions::{self as sup, Suppression, SuppressionInput};
use crate::state::AppState;

/// Top-level rule fields we model; the others are kept as they were sent.
const RULE_FIELDS: [&str; 22] = [
    "id", "version", "name", "message", "type", "isEnabled", "isDefault", "queries", "cases", "options", "filters", "tags", "hasExtendedTitle", "createdAt", "updatedAt",
    "creationAuthorId", "updateAuthorId", "isPartner", "isBeta", "isDeleted", "isDeprecated", "defaultTags",
];

fn rule_doc(r: &SecurityRule) -> Value {
    let mut doc = json!({
        "id": r.id, "version": r.version, "name": r.name, "message": r.message, "type": r.kind,
        "createdAt": r.created_at, "updatedAt": r.updated_at, "creationAuthorId": r.creation_author_id, "updateAuthorId": r.update_author_id,
        "isDefault": r.is_default, "isPartner": false, "isEnabled": r.is_enabled, "isBeta": false, "isDeleted": false, "isDeprecated": false,
        "queries": r.queries, "options": r.options, "cases": r.cases, "filters": r.filters, "tags": r.tags, "defaultTags": [],
        "hasExtendedTitle": r.has_extended_title,
    });
    if let (Some(d), Some(extra)) = (doc.as_object_mut(), r.extra.as_object()) {
        for (k, v) in extra {
            d.entry(k.clone()).or_insert(v.clone());
        }
    }
    doc
}

/// A rule body as our input, plus the fields we keep without interpreting.
fn rule_input(v: &Value) -> Result<(SecurityRuleInput, Value), DdError> {
    let obj = v.as_object().ok_or_else(|| bad_request("The request body must be a rule object"))?;
    let input: SecurityRuleInput = serde_json::from_value(v.clone()).map_err(|e| DdError::new(StatusCode::BAD_REQUEST, format!("Invalid rule configuration: {e}")))?;
    let extra: Map<String, Value> = obj.iter().filter(|(k, _)| !RULE_FIELDS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok((input, Value::Object(extra)))
}

fn invalid(m: String) -> DdError {
    DdError { status: StatusCode::BAD_REQUEST, errors: vec!["Invalid rule configuration".into(), m] }
}

/// A rule id of ours (`abc-def-ghi`), else not found.
fn rule_id(id: &str) -> Result<&str, DdError> {
    let ok = id.len() == 11 && id.split('-').count() == 3 && id.split('-').all(|p| p.len() == 3 && p.chars().all(|c| c.is_ascii_alphanumeric()));
    if ok {
        Ok(id)
    } else {
        Err(not_found("Not found"))
    }
}

async fn find_rule(st: &AppState, ctx: &AuthContext, id: &str) -> Result<SecurityRule, DdError> {
    load_rule(st, &ctx.org_id, rule_id(id)?).await.map_err(|_| not_found(format!("Threat detection rule not found: {id}")))
}

/// List rules (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/security_monitoring/rules", operation_id = "datadog_list_security_rules", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_rules(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> DocResult {
    let rules = load_rules(&st, &ctx.org_id).await?;
    let size: usize = params.get("page[size]").and_then(|s| s.parse().ok()).unwrap_or(10).clamp(1, 1000);
    let number: usize = params.get("page[number]").and_then(|s| s.parse().ok()).unwrap_or(0);
    let total = rules.len();
    let page: Vec<Value> = rules.iter().skip(number * size).take(size).map(rule_doc).collect();
    ok(json!({ "data": page, "meta": { "page": { "total_count": total, "total_filtered_count": total } } }))
}

#[utoipa::path(post, path = "/api/v2/security_monitoring/rules", request_body = Object, operation_id = "datadog_create_security_rule", responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let (input, extra) = rule_input(&body(&raw)?)?;
    input.validate().map_err(invalid)?;
    let id = insert_rule(&st, &ctx, &input, extra).await?;
    ok(rule_doc(&load_rule(&st, &ctx.org_id, &id).await?))
}

/// Validate a rule without saving it.
#[utoipa::path(post, path = "/api/v2/security_monitoring/rules/validation", request_body = Object, operation_id = "datadog_validate_security_rule", responses((status = 204), (status = 400, body = Object)), tag = "datadog-api")]
pub async fn validate_rule(DdAuth(ctx): DdAuth, raw: Bytes) -> Result<StatusCode, (StatusCode, axum::Json<Value>)> {
    let fail = |m: String, target: &str| (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": { "code": "InvalidArgument", "message": "Invalid rule configuration", "details": [{ "code": "InvalidArgument", "message": m, "target": target }] } })));
    let _ = ctx;
    let v: Value = serde_json::from_slice(&raw).map_err(|e| fail(format!("Invalid JSON body: {e}"), ""))?;
    let (input, _) = rule_input(&v).map_err(|e| fail(e.errors.join(": "), ""))?;
    input.validate().map_err(|m| {
        let target = if m.contains("Keep alive") { "keepAlive" } else if m.contains("Max signal") { "maxSignalDuration" } else if m.contains("Evaluation") { "evaluationWindow" } else { "" };
        fail(m, target)
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/security_monitoring/rules/{rule_id}", operation_id = "datadog_get_security_rule", params(("rule_id" = String, Path)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(rule_doc(&find_rule(&st, &ctx, &id).await?))
}

#[utoipa::path(put, path = "/api/v2/security_monitoring/rules/{rule_id}", request_body = Object, operation_id = "datadog_update_security_rule", params(("rule_id" = String, Path)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let current = find_rule(&st, &ctx, &id).await?;
    let changes = body(&raw)?;
    let changes = changes.as_object().ok_or_else(|| bad_request("The request body must be a rule object"))?;
    // The fields sent replace the rule's.
    let mut merged = rule_doc(&current);
    for (k, v) in changes {
        merged[k] = v.clone();
    }
    let (input, extra) = rule_input(&merged)?;
    if !current.is_default {
        input.validate().map_err(invalid)?;
    }
    update_rule_row(&st, &ctx, &current.id, &input, extra).await?;
    ok(rule_doc(&load_rule(&st, &ctx.org_id, &current.id).await?))
}

#[utoipa::path(delete, path = "/api/v2/security_monitoring/rules/{rule_id}", operation_id = "datadog_delete_security_rule", params(("rule_id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    let current = find_rule(&st, &ctx, &id).await?;
    delete_rule_row(&st, &ctx.org_id, &current.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ signals

fn signal_doc(s: &SecuritySignal) -> Value {
    let iso = |ms: i64| super::users::iso(ms);
    json!({
        "id": s.id,
        "type": "signal",
        "attributes": {
            "timestamp": iso(s.created_at),
            "message": s.message,
            "tags": s.tags,
            "custom": {
                "title": s.title,
                "status": s.severity,
                "source": s.source,
                "service": s.services,
                "env": s.env,
                "group": s.group_values,
                "entities": s.entities,
                "events_matched": s.events_matched,
                "first_seen": iso(s.first_seen),
                "last_seen": iso(s.last_seen),
                "workflow": {
                    "rule": { "id": s.rule_id, "name": s.rule_name, "type": s.rule_type },
                    "triggeringRuleCase": { "name": s.case_name, "status": s.severity, "condition": s.condition },
                    "triage": {
                        "state": s.state,
                        "archiveReason": s.archive_reason,
                        "archiveComment": s.archive_comment,
                        "assignee": s.assignee.as_ref().map_or(json!({ "id": -1, "name": "Unassigned" }), |a| json!({ "uuid": a.id, "name": a.name, "handle": a.email })),
                        "incidentIds": s.incident_ids,
                    },
                },
            },
        },
    })
}

fn metadata_doc(s: &SecuritySignal) -> Value {
    json!({
        "data": {
            "id": s.id,
            "type": "signal_metadata",
            "attributes": {
                "state": s.state,
                "archive_reason": s.archive_reason,
                "archive_comment": s.archive_comment,
                "assignee": s.assignee.as_ref().map_or(json!({ "id": -1, "name": "Unassigned", "uuid": "" }), |a| json!({ "uuid": a.id, "name": a.name, "handle": a.email })),
                "incident_ids": s.incident_ids,
                "state_update_timestamp": s.state_updated_at,
                "state_update_user": s.state_updated_by.as_ref().map(|n| json!({ "name": n })),
            },
        },
    })
}

fn parse_time(v: Option<&String>, default: i64) -> Result<i64, DdError> {
    match v {
        None => Ok(default),
        Some(s) if s.chars().all(|c| c.is_ascii_digit()) => s.parse().map_err(|_| bad_request(format!("invalid time {s}"))),
        Some(s) => chrono::DateTime::parse_from_rfc3339(s).map(|t| t.timestamp_millis()).map_err(|_| bad_request(format!("invalid time {s}: use RFC 3339"))),
    }
}

/// Searches signals: Datadog's filter (query, from, to), sort and cursor paging.
#[allow(clippy::too_many_arguments)]
async fn signal_page(st: &AppState, ctx: &AuthContext, query: &str, from: i64, to: i64, ascending: bool, cursor: Option<&str>, limit: usize) -> DocResult {
    let all = signals_in(st, &ctx.org_id, TimeRange { from, to }).await?;
    let terms = oc_query::parse_query(query);
    let mut shown: Vec<&SecuritySignal> = all.iter().filter(|s| s.created_at >= from && matches(s, &terms)).collect();
    if ascending {
        shown.reverse();
    }
    let offset: usize = cursor
        .and_then(|c| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(c).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let limit = limit.clamp(1, 1000);
    let page: Vec<Value> = shown.iter().skip(offset).take(limit).map(|s| signal_doc(s)).collect();
    let mut doc = json!({ "data": page });
    if offset + limit < shown.len() {
        let next = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode((offset + limit).to_string());
        doc["meta"] = json!({ "page": { "after": next } });
    }
    ok(doc)
}

#[utoipa::path(get, path = "/api/v2/security_monitoring/signals", operation_id = "datadog_list_security_signals", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_signals(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> DocResult {
    let now = chrono::Utc::now().timestamp_millis();
    let from = parse_time(params.get("filter[from]"), now - 15 * 60_000)?;
    let to = parse_time(params.get("filter[to]"), now)?;
    let limit = params.get("page[limit]").and_then(|s| s.parse().ok()).unwrap_or(10);
    signal_page(&st, &ctx, params.get("filter[query]").map_or("", String::as_str), from, to, params.get("sort").is_some_and(|s| s == "timestamp"), params.get("page[cursor]").map(String::as_str), limit).await
}

#[utoipa::path(post, path = "/api/v2/security_monitoring/signals/search", request_body = Object, operation_id = "datadog_search_security_signals", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn search_signals(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    let v = if raw.is_empty() { json!({}) } else { body(&raw)? };
    let now = chrono::Utc::now().timestamp_millis();
    let s = |p: &str| v.pointer(p).and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string())));
    let from = parse_time(s("/filter/from").as_ref(), now - 15 * 60_000)?;
    let to = parse_time(s("/filter/to").as_ref(), now)?;
    let limit = v.pointer("/page/limit").and_then(Value::as_u64).unwrap_or(10) as usize;
    signal_page(&st, &ctx, &s("/filter/query").unwrap_or_default(), from, to, s("/sort").as_deref() == Some("timestamp"), s("/page/cursor").as_deref(), limit).await
}

async fn find_signal(st: &AppState, ctx: &AuthContext, id: &str) -> Result<SecuritySignal, DdError> {
    load_signal(st, &ctx.org_id, id).await.map_err(|_| not_found("Signal not found"))
}

#[utoipa::path(get, path = "/api/v2/security_monitoring/signals/{signal_id}", operation_id = "datadog_get_security_signal", params(("signal_id" = String, Path)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_signal(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": signal_doc(&find_signal(&st, &ctx, &id).await?) }))
}

fn attributes(raw: &Bytes) -> Result<Value, DdError> {
    let v = body(raw)?;
    Ok(data(&v)?.get("attributes").cloned().unwrap_or_else(|| json!({})))
}

#[utoipa::path(patch, path = "/api/v2/security_monitoring/signals/{signal_id}/state", request_body = Object, operation_id = "datadog_set_security_signal_state", params(("signal_id" = String, Path)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn signal_state(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let signal = find_signal(&st, &ctx, &id).await?;
    let a = attributes(&raw)?;
    let input = SignalStateInput {
        state: a.get("state").and_then(Value::as_str).unwrap_or_default().to_string(),
        archive_reason: a.get("archive_reason").and_then(Value::as_str).map(str::to_string),
        archive_comment: a.get("archive_comment").and_then(Value::as_str).map(str::to_string),
    };
    set_state(&st, &ctx, std::slice::from_ref(&signal.id), &input).await.map_err(|e| bad_request(e.to_string()))?;
    ok(metadata_doc(&load_signal(&st, &ctx.org_id, &signal.id).await?))
}

#[utoipa::path(patch, path = "/api/v2/security_monitoring/signals/{signal_id}/assignee", request_body = Object, operation_id = "datadog_set_security_signal_assignee", params(("signal_id" = String, Path)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn signal_assignee(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let signal = find_signal(&st, &ctx, &id).await?;
    let a = attributes(&raw)?;
    let uuid = a.pointer("/assignee/uuid").and_then(Value::as_str).filter(|u| !u.is_empty()).map(str::to_string);
    set_assignee(&st, &ctx, std::slice::from_ref(&signal.id), uuid.as_deref()).await.map_err(|e| bad_request(e.to_string()))?;
    ok(metadata_doc(&load_signal(&st, &ctx.org_id, &signal.id).await?))
}

#[utoipa::path(patch, path = "/api/v2/security_monitoring/signals/{signal_id}/incidents", request_body = Object, operation_id = "datadog_set_security_signal_incidents", params(("signal_id" = String, Path)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn signal_incidents(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let signal = find_signal(&st, &ctx, &id).await?;
    let a = attributes(&raw)?;
    let ids: Vec<i64> = a.get("incident_ids").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
    sqlx::query("UPDATE security_signals SET incident_ids = $3 WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(&signal.id).bind(&ids).execute(&st.postgres).await.map_err(crate::error::ApiError::from)?;
    ok(metadata_doc(&load_signal(&st, &ctx.org_id, &signal.id).await?))
}

// ================================================================ suppressions

fn suppression_doc(s: &Suppression) -> Value {
    let mut tags = s.tags.clone();
    tags.sort();
    let mut attrs = json!({
        "name": s.name, "description": s.description, "enabled": s.enabled, "rule_query": s.rule_query, "suppression_query": s.suppression_query,
        "data_exclusion_query": s.data_exclusion_query, "tags": tags, "version": s.version, "editable": true,
        "creation_date": s.created_at, "update_date": s.updated_at,
        "creator": { "handle": s.creator, "name": s.creator }, "updater": { "handle": s.updater, "name": s.updater },
    });
    if let Some(d) = s.start_date {
        attrs["start_date"] = json!(d);
    }
    if let Some(d) = s.expiration_date {
        attrs["expiration_date"] = json!(d);
    }
    json!({ "id": s.id, "type": "suppressions", "attributes": attrs })
}

/// A suppression body's attributes over `base` (an update changes the attributes it sends).
fn suppression_input(raw: &Bytes, base: SuppressionInput) -> Result<SuppressionInput, DdError> {
    let v = body(raw)?;
    let d = data(&v)?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != "suppressions") {
        return Err(bad_request("`data.type` must be suppressions"));
    }
    let a = d.get("attributes").cloned().unwrap_or_else(|| json!({}));
    let s = |k: &str, cur: String| -> Result<String, DdError> {
        match a.get(k) {
            None => Ok(cur),
            Some(Value::Null) => Ok(String::new()),
            Some(Value::String(v)) => Ok(v.clone()),
            Some(_) => Err(bad_request(format!("input_validation_error(Field 'data.attributes.{k}' must be a string)"))),
        }
    };
    let n = |k: &str, cur: Option<i64>| -> Option<i64> {
        match a.get(k) {
            None => cur,
            Some(v) => v.as_i64(),
        }
    };
    let input = SuppressionInput {
        name: s("name", base.name)?,
        description: s("description", base.description)?,
        enabled: a.get("enabled").and_then(Value::as_bool).unwrap_or(base.enabled),
        rule_query: s("rule_query", base.rule_query)?,
        suppression_query: s("suppression_query", base.suppression_query)?,
        data_exclusion_query: s("data_exclusion_query", base.data_exclusion_query)?,
        start_date: n("start_date", base.start_date),
        expiration_date: n("expiration_date", base.expiration_date),
        tags: a.get("tags").and_then(Value::as_array).map(|t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or(base.tags),
    };
    check_queries(&input)?;
    Ok(input)
}

/// The queries must parse: balanced brackets and quotes.
fn check_queries(i: &SuppressionInput) -> Result<(), DdError> {
    for (k, q) in [("rule_query", &i.rule_query), ("suppression_query", &i.suppression_query), ("data_exclusion_query", &i.data_exclusion_query)] {
        let balanced = q.matches('(').count() == q.matches(')').count() && q.matches('[').count() == q.matches(']').count() && q.matches('"').count() % 2 == 0;
        if !balanced {
            return Err(bad_request(format!("input_validation_error(Field 'data.attributes.{k}' is invalid: {} is invalid)", k.replace('_', " "))));
        }
    }
    i.validate().map_err(|m| bad_request(format!("input_validation_error({m})")))
}

fn input_of(s: &Suppression) -> SuppressionInput {
    SuppressionInput {
        name: s.name.clone(),
        description: s.description.clone(),
        enabled: s.enabled,
        rule_query: s.rule_query.clone(),
        suppression_query: s.suppression_query.clone(),
        data_exclusion_query: s.data_exclusion_query.clone(),
        start_date: s.start_date,
        expiration_date: s.expiration_date,
        tags: s.tags.clone(),
    }
}

#[utoipa::path(get, path = "/api/v2/security_monitoring/configuration/suppressions", operation_id = "datadog_list_suppressions", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list_suppressions(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> DocResult {
    let mut rows = sup::list_rows(&st, &ctx.org_id).await?;
    // `query`: `id:a OR id:b`, or text in the name.
    if let Some(q) = params.get("query").filter(|q| !q.trim().is_empty()) {
        let ids: Vec<&str> = q.split(" OR ").filter_map(|t| t.trim().strip_prefix("id:")).collect();
        rows.retain(|s| if ids.is_empty() { s.name.to_lowercase().contains(&q.to_lowercase()) } else { ids.contains(&s.id.as_str()) });
    }
    match params.get("sort").map(String::as_str) {
        Some("-name") => rows.sort_by(|a, b| b.name.cmp(&a.name)),
        Some("start_date") => rows.sort_by_key(|s| s.start_date),
        Some("-start_date") => rows.sort_by_key(|s| std::cmp::Reverse(s.start_date)),
        Some("expiration_date") => rows.sort_by_key(|s| s.expiration_date),
        Some("-expiration_date") => rows.sort_by_key(|s| std::cmp::Reverse(s.expiration_date)),
        Some("updated_at") => rows.sort_by_key(|s| s.updated_at),
        Some("-updated_at") => rows.sort_by_key(|s| std::cmp::Reverse(s.updated_at)),
        _ => rows.sort_by(|a, b| a.name.cmp(&b.name)),
    }
    let total = rows.len();
    let docs: Vec<Value> = match params.get("page[size]").and_then(|s| s.parse::<usize>().ok()) {
        Some(size) => {
            let number: usize = params.get("page[number]").and_then(|s| s.parse().ok()).unwrap_or(0);
            rows.iter().skip(number * size.max(1)).take(size.max(1)).map(suppression_doc).collect()
        }
        None => rows.iter().map(suppression_doc).collect(),
    };
    let number: usize = params.get("page[number]").and_then(|s| s.parse().ok()).unwrap_or(0);
    ok(json!({ "data": docs, "meta": { "page": { "totalCount": total, "pageSize": docs.len(), "pageNumber": number } } }))
}

#[utoipa::path(post, path = "/api/v2/security_monitoring/configuration/suppressions", request_body = Object, operation_id = "datadog_create_suppression",
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create_suppression(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let input = suppression_input(&raw, SuppressionInput { enabled: true, ..Default::default() })?;
    let id = sup::insert(&st, &ctx, &input).await?;
    ok(json!({ "data": suppression_doc(&sup::load(&st, &ctx.org_id, &id).await?) }))
}

#[utoipa::path(post, path = "/api/v2/security_monitoring/configuration/suppressions/validation", request_body = Object, operation_id = "datadog_validate_suppression",
    responses((status = 204), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn validate_suppression(DdAuth(ctx): DdAuth, raw: Bytes) -> Result<StatusCode, DdError> {
    let _ = ctx;
    suppression_input(&raw, SuppressionInput { enabled: true, ..Default::default() })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn find_suppression(st: &AppState, ctx: &AuthContext, id: &str) -> Result<Suppression, DdError> {
    sup::load(st, &ctx.org_id, id).await.map_err(|_| not_found(format!("Suppression not found: {id}")))
}

#[utoipa::path(get, path = "/api/v2/security_monitoring/configuration/suppressions/{suppression_id}", operation_id = "datadog_get_suppression", params(("suppression_id" = String, Path)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_suppression(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": suppression_doc(&find_suppression(&st, &ctx, &id).await?) }))
}

#[utoipa::path(patch, path = "/api/v2/security_monitoring/configuration/suppressions/{suppression_id}", request_body = Object, operation_id = "datadog_update_suppression", params(("suppression_id" = String, Path)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors), (status = 409, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update_suppression(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let current = find_suppression(&st, &ctx, &id).await?;
    // `version`: the version the client read (concurrent modification otherwise).
    let sent_version = body(&raw)?.pointer("/data/attributes/version").and_then(Value::as_i64);
    if sent_version.is_some_and(|v| v != current.version as i64) {
        return Err(DdError::new(StatusCode::CONFLICT, "Concurrent Modification: the suppression changed since it was read"));
    }
    let input = suppression_input(&raw, input_of(&current))?;
    sup::update_row(&st, &ctx, &current.id, &input).await?;
    ok(json!({ "data": suppression_doc(&sup::load(&st, &ctx.org_id, &current.id).await?) }))
}

#[utoipa::path(delete, path = "/api/v2/security_monitoring/configuration/suppressions/{suppression_id}", operation_id = "datadog_delete_suppression", params(("suppression_id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_suppression(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    let current = find_suppression(&st, &ctx, &id).await?;
    sup::delete_row(&st, &ctx.org_id, &current.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::super::replay::{Feature, TestServer};
    use super::*;

    #[test]
    fn rule_bodies_keep_unknown_fields() {
        let v = json!({"name": "r", "queries": [{"query": "source:okta"}], "cases": [{"status": "info", "condition": "a > 0"}],
                       "referenceTables": [{"tableName": "t"}], "isEnabled": true, "type": "log_detection"});
        let (input, extra) = rule_input(&v).unwrap();
        assert_eq!(input.queries[0].aggregation, "count");
        assert_eq!(extra, json!({"referenceTables": [{"tableName": "t"}]}));
        assert!(rule_id("abc-def-ghi").is_ok());
        assert!(rule_id("abcde-12345").is_err());
    }

    #[test]
    fn suppression_queries_must_be_balanced() {
        let raw = Bytes::from(r#"{"data":{"attributes":{"data_exclusion_query":"not enough attributes","enabled":false,"name":"cold_harbour","rule_query":"rule:[A-Invalid"},"type":"suppressions"}}"#);
        assert!(suppression_input(&raw, SuppressionInput::default()).unwrap_err().errors[0].contains("rule_query"));
        let raw = Bytes::from(r#"{"data":{"attributes":{"name":"x","rule_query":"source:cloudtrail","suppression_query":"env:staging"},"type":"suppressions"}}"#);
        let i = suppression_input(&raw, SuppressionInput { enabled: true, ..Default::default() }).unwrap();
        assert!(i.enabled);
    }

    #[tokio::test]
    async fn replays_datadogs_security_monitoring_scenarios() {
        let Some(f) = Feature::load("v2/security_monitoring") else { return };
        let Some(s) = TestServer::start("dd-security-replay").await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            // The recorded list starts with a cloud configuration rule, whose compliance fields our
            // log detection rules don't have.
            problems.extend(p.into_iter().filter(|m| !m.contains("compliance")));
            replayed += 1;
        }
        assert!(replayed >= 15, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
