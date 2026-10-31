//! Log pipelines, Datadog-compatible (`/api/v1/logs/config/pipelines`, `/pipeline-order`): the
//! provider's `datadog_logs_custom_pipeline`, `datadog_logs_integration_pipeline` and
//! `datadog_logs_pipeline_order`, on `log_pipelines` ([`crate::routes::log_pipelines`]).
//!
//! - Processors are stored as the UI writes them (camelCase keys, `enabled`) and answered in
//!   Datadog's form (snake_case keys, `is_enabled`); the engine reads both. Processor types the
//!   engine doesn't run (array processors, decoders, schema processors…) are kept and skipped.
//! - Integration pipelines answer `type: integration` and `is_read_only: true`; only `is_enabled`
//!   changes on them, as Datadog does.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Map, Value};

use super::super::users::{bad_request, body, not_found, ok, require, uuid, DocResult};
use super::super::{DdAuth, DdError};
use super::{dd_query, our_query};
use crate::auth::Access;
use crate::routes::log_pipelines::{insert, validate, LogPipelineInput};
use crate::state::AppState;

/// Key renames between Datadog's processors and ours that aren't a plain case change.
const RENAMES: [(&str, &str); 2] = [("is_enabled", "enabled"), ("is_replace_missing", "replaceMissing")];

fn camel(k: &str) -> String {
    if let Some((_, ours)) = RENAMES.iter().find(|(dd, _)| *dd == k) {
        return ours.to_string();
    }
    let mut out = String::with_capacity(k.len());
    let mut up = false;
    for c in k.chars() {
        if c == '_' && !out.is_empty() {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn snake(k: &str) -> String {
    if let Some((dd, _)) = RENAMES.iter().find(|(_, ours)| *ours == k) {
        return dd.to_string();
    }
    let mut out = String::with_capacity(k.len() + 4);
    for c in k.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn rekey(v: &Value, f: &dyn Fn(&str) -> String) -> Value {
    match v {
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (f(k), rekey(v, f))).collect::<Map<_, _>>()),
        Value::Array(a) => Value::Array(a.iter().map(|v| rekey(v, f)).collect()),
        v => v.clone(),
    }
}

/// The processor types the UI edits (and the engine runs): stored with our keys. Others are stored
/// as Datadog sent them, since their maps can have free-form keys (schema mappers' attribute paths).
const OURS: [&str; 13] = [
    "grok-parser",
    "status-remapper",
    "date-remapper",
    "service-remapper",
    "message-remapper",
    "trace-id-remapper",
    "attribute-remapper",
    "category-processor",
    "string-builder-processor",
    "lookup-processor",
    "arithmetic-processor",
    "url-parser",
    "pipeline",
];

fn ours(p: &Value) -> bool {
    p.get("type").and_then(Value::as_str).is_some_and(|t| OURS.contains(&t))
}

/// Renames a processor's keys; a nested pipeline's processors are converted one by one.
fn convert(p: &Value, f: &dyn Fn(&str) -> String, inner: &dyn Fn(&[Value]) -> Vec<Value>) -> Value {
    let Some(o) = p.as_object() else { return p.clone() };
    Value::Object(
        o.iter()
            .map(|(k, v)| match (k.as_str(), v) {
                ("processors", Value::Array(items)) => (k.clone(), Value::Array(inner(items))),
                _ => (f(k), rekey(v, f)),
            })
            .collect(),
    )
}

/// Datadog's processors → what we store.
pub(crate) fn to_ours(processors: &[Value]) -> Vec<Value> {
    processors.iter().map(|p| if ours(p) { convert(p, &camel, &to_ours) } else { p.clone() }).collect()
}

/// Fields Datadog answers with their defaults when they weren't sent.
fn defaults(p: &mut Value) {
    let Some(o) = p.as_object_mut() else { return };
    fn set(o: &mut Map<String, Value>, k: &str, v: Value) {
        o.entry(k).or_insert(v);
    }
    let ty = o.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    match ty.as_str() {
        "attribute-remapper" => {
            set(o, "preserve_source", json!(false));
            set(o, "override_on_conflict", json!(false));
            set(o, "source_type", json!("attribute"));
            set(o, "target_type", json!("attribute"));
        }
        "string-builder-processor" | "arithmetic-processor" => set(o, "is_replace_missing", json!(false)),
        "pipeline" => set(o, "processors", json!([])),
        "array-map-processor" => set(o, "preserve_source", json!(true)),
        "array-processor" => {
            if let Some(op) = o.get_mut("operation").and_then(Value::as_object_mut) {
                match op.get("type").and_then(Value::as_str) {
                    Some("append") => {
                        op.entry("preserve_source").or_insert(json!(true));
                    }
                    Some("key-value") => {
                        op.entry("override_on_conflict").or_insert(json!(false));
                    }
                    _ => {}
                }
            }
        }
        "schema-processor" => {
            if let Some(schema) = o.get_mut("schema").and_then(Value::as_object_mut) {
                schema.entry("extensions").or_insert(json!([]));
            }
            for m in o.get_mut("mappers").and_then(Value::as_array_mut).into_iter().flatten() {
                let Some(m) = m.as_object_mut() else { continue };
                match m.get("type").and_then(Value::as_str) {
                    Some("schema-category-mapper") => {
                        m.entry("fallback").or_insert(json!({ "values": {}, "sources": {} }));
                    }
                    Some("schema-remapper") => {
                        m.entry("preserve_source").or_insert(json!(false));
                        m.entry("override_on_conflict").or_insert(json!(false));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if let Some(inner) = o.get_mut("processors").and_then(Value::as_array_mut) {
        inner.iter_mut().for_each(defaults);
    }
}

/// What we store → Datadog's processors (`is_enabled` always present, as Datadog answers).
pub(crate) fn to_datadog(processors: &[Value]) -> Vec<Value> {
    processors
        .iter()
        .map(|p| {
            let mut d = if ours(p) { convert(p, &snake, &to_datadog) } else { p.clone() };
            if let Some(o) = d.as_object_mut() {
                o.entry("is_enabled").or_insert(Value::Bool(true));
            }
            defaults(&mut d);
            d
        })
        .collect()
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    enabled: bool,
    filter: String,
    processors: Value,
    source: String,
    description: String,
    tags: Value,
}

const SELECT: &str = "SELECT id::text AS id, name, enabled, filter, processors, source, description, tags FROM log_pipelines";

fn doc(r: &Row) -> Value {
    let integration = r.source != "custom";
    let processors = r.processors.as_array().cloned().unwrap_or_default();
    let mut d = json!({
        "id": r.id,
        "type": if integration { "integration" } else { "pipeline" },
        "name": r.name,
        "is_enabled": r.enabled,
        "is_read_only": integration,
        "filter": { "query": dd_query(&r.filter) },
        "processors": to_datadog(&processors),
        "tags": r.tags,
    });
    if !r.description.is_empty() {
        d["description"] = Value::from(r.description.clone());
    }
    d
}

async fn find(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    let Some(id) = uuid(id) else { return Err(bad_request(format!("Invalid pipeline id {id}"))) };
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?.ok_or_else(|| not_found("Pipeline not found"))
}

struct Input {
    pipeline: LogPipelineInput,
    description: String,
    tags: Vec<String>,
}

fn input(v: &Value) -> Result<Input, DdError> {
    if !v.is_object() {
        return Err(bad_request("The body must be a pipeline object"));
    }
    let name = v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let filter = our_query(v.get("filter").and_then(|f| f.get("query")).and_then(Value::as_str).unwrap_or(""));
    let processors = match v.get("processors") {
        Some(Value::Array(p)) => to_ours(p),
        Some(Value::Null) | None => vec![],
        Some(_) => return Err(bad_request("processors must be a list")),
    };
    let mut pipeline = LogPipelineInput { name, filter, enabled: v.get("is_enabled").and_then(Value::as_bool).unwrap_or(false), processors };
    validate(&mut pipeline).map_err(DdError::from)?;
    Ok(Input {
        pipeline,
        description: v.get("description").and_then(Value::as_str).unwrap_or("").chars().take(1000).collect(),
        tags: v.get("tags").and_then(Value::as_array).map(|t| t.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
    })
}

pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 ORDER BY position, created_at")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    ok(Value::Array(rows.iter().map(doc).collect()))
}

pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(doc(&find(&st, &ctx.org_id, &id).await?))
}

pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let i = input(&body(&raw)?)?;
    let id = insert(&st, &ctx.org_id, &i.pipeline, "custom").await.map_err(DdError::from)?.ok_or_else(|| bad_request("The pipeline could not be created"))?;
    sqlx::query("UPDATE log_pipelines SET description = $3, tags = $4, updated_by = $5::uuid WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&i.description)
        .bind(serde_json::to_value(&i.tags).unwrap_or_default())
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "log_pipeline.created", &i.pipeline.name, "API").await;
    ok(doc(&find(&st, &ctx.org_id, &id).await?))
}

/// Replaces a custom pipeline; on an integration pipeline only `is_enabled` changes.
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let current = find(&st, &ctx.org_id, &id).await?;
    let v = body(&raw)?;
    if current.source != "custom" {
        let enabled = v.get("is_enabled").and_then(Value::as_bool).unwrap_or(current.enabled);
        sqlx::query("UPDATE log_pipelines SET enabled = $3, updated_by = $4::uuid, updated_at = now() WHERE org_id = $1 AND id::text = $2")
            .bind(&ctx.org_id)
            .bind(&current.id)
            .bind(enabled)
            .bind(&ctx.user_id)
            .execute(&st.postgres)
            .await?;
    } else {
        let i = input(&v)?;
        sqlx::query(
            "UPDATE log_pipelines SET name = $3, enabled = $4, filter = $5, processors = $6, description = $7, tags = $8, updated_by = $9::uuid, updated_at = now() \
             WHERE org_id = $1 AND id::text = $2",
        )
        .bind(&ctx.org_id)
        .bind(&current.id)
        .bind(&i.pipeline.name)
        .bind(i.pipeline.enabled)
        .bind(&i.pipeline.filter)
        .bind(Value::Array(i.pipeline.processors.clone()))
        .bind(&i.description)
        .bind(serde_json::to_value(&i.tags).unwrap_or_default())
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    }
    ctx.audit(&st, "log_pipeline.updated", &current.name, "API").await;
    ok(doc(&find(&st, &ctx.org_id, &current.id).await?))
}

pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    require(&ctx, Access::Standard)?;
    let p = find(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_pipelines WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&p.id).execute(&st.postgres).await?;
    ctx.audit(&st, "log_pipeline.deleted", &p.name, "API").await;
    ok(json!({}))
}

async fn order(st: &AppState, org: &str) -> Result<Vec<String>, DdError> {
    Ok(sqlx::query_scalar("SELECT id::text FROM log_pipelines WHERE org_id = $1 ORDER BY position, created_at").bind(org).fetch_all(&st.postgres).await?)
}

pub async fn get_order(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    ok(json!({ "pipeline_ids": order(&st, &ctx.org_id).await? }))
}

/// Every pipeline, once, in processing order.
pub async fn update_order(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let v = body(&raw)?;
    let ids: Vec<String> = v.get("pipeline_ids").and_then(Value::as_array).ok_or_else(|| bad_request("The parameter 'pipeline_ids' is required"))?.iter().filter_map(Value::as_str).map(str::to_string).collect();
    let current = order(&st, &ctx.org_id).await?;
    let (mut a, mut b) = (ids.clone(), current);
    a.sort();
    b.sort();
    if a != b {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "pipeline_ids must list every pipeline of the organization exactly once"));
    }
    let mut tx = st.postgres.begin().await?;
    for (pos, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE log_pipelines SET position = $3 WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(id).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "log_pipeline.reordered", "", "API").await;
    ok(json!({ "pipeline_ids": ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processors_round_trip_between_datadog_and_our_keys() {
        let dd = vec![
            json!({"type": "grok-parser", "is_enabled": false, "source": "message", "samples": [], "grok": {"match_rules": "r %{word:w}", "support_rules": ""}}),
            json!({"type": "string-builder-processor", "is_enabled": true, "template": "x", "target": "y", "is_replace_missing": true}),
            json!({"type": "array-processor", "is_enabled": true, "operation": {"type": "append", "source": "network.client.ip", "target": "sourceIps", "preserve_source": false}}),
            json!({"type": "pipeline", "is_enabled": true, "filter": {"query": "env:prod"}, "processors": [{"type": "status-remapper", "is_enabled": true, "sources": ["lvl"]}]}),
        ];
        let ours = to_ours(&dd);
        assert_eq!(ours[0]["grok"]["matchRules"], "r %{word:w}");
        assert_eq!(ours[0]["enabled"], false);
        assert_eq!(ours[1]["replaceMissing"], true);
        // Processors the UI doesn't edit are kept as sent (their maps may have free-form keys).
        assert_eq!(ours[2], dd[2]);
        assert_eq!(ours[3]["processors"][0]["enabled"], true);
        let back = to_datadog(&ours);
        assert_eq!(back[0], dd[0]);
        assert_eq!(back[1], dd[1]);
        assert_eq!(back[2], dd[2]);
        assert_eq!(back[3]["processors"][0]["sources"], json!(["lvl"]));
        // The UI's processors (no enabled flag) are answered enabled, with Datadog's defaults.
        assert_eq!(to_datadog(&[json!({"type": "url-parser", "sources": ["u"], "target": "t"})])[0]["is_enabled"], true);
        let r = &to_datadog(&[json!({"type": "attribute-remapper", "sources": ["a"], "target": "b"})])[0];
        assert_eq!((r["preserve_source"].clone(), r["source_type"].clone()), (json!(false), json!("attribute")));
    }
}
