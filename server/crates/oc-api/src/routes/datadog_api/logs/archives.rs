//! Log archives, Datadog-compatible (`/api/v2/logs/config/archives`, `/archive-order`): the provider's
//! `datadog_logs_archive` and `datadog_logs_archive_order`, on `log_archives` ([`crate::routes::archives`]).
//!
//! - Datadog's `destination` (`s3`, `gcs`, `azure`) is kept as sent. An `s3` destination writes to AWS
//!   S3 and a `gcs` one to Cloud Storage's S3-compatible API; `azure` isn't written to.
//! - Datadog's API carries no secrets (it uses its cloud integrations), so an archive created here
//!   archives nothing until its access and secret keys are set in Logs → Configuration → Archiving
//!   (`state` is `UNKNOWN` until then). Archives created in the UI are answered as `s3` destinations.
//! - `rehydration_max_scan_size_in_gb` caps rehydrations; `include_tags` and `rehydration_tags` are
//!   kept (tags are always archived).

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::super::users::{bad_request, body, data, not_found, ok, require, uuid, DocResult};
use super::super::{DdAuth, DdError};
use super::{dd_query, our_query};
use crate::auth::Access;
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    endpoint: String,
    bucket: String,
    prefix: String,
    access_key: String,
    query: String,
    destination: Option<Value>,
    include_tags: bool,
    rehydration_tags: Value,
    rehydration_max_scan_size_gb: Option<i64>,
    has_secret: bool,
    failing: bool,
}

const SELECT: &str = "SELECT a.id::text AS id, a.name, a.endpoint, a.bucket, a.prefix, a.access_key, a.query, a.destination, a.include_tags, a.rehydration_tags, \
    a.rehydration_max_scan_size_gb, a.secret_key_enc <> '' AS has_secret, \
    COALESCE((SELECT r.status = 'failed' FROM archive_runs r WHERE r.archive_id = a.id ORDER BY r.finished_at DESC LIMIT 1), false) AS failing \
    FROM log_archives a";

fn destination(r: &Row) -> Value {
    if let Some(d) = &r.destination {
        return d.clone();
    }
    json!({
        "type": "s3",
        "bucket": r.bucket,
        "path": format!("/{}", r.prefix),
        "integration": { "access_key_id": r.access_key },
        "encryption": { "type": "NO_OVERRIDE" },
        "storage_class": "STANDARD",
    })
}

fn state(r: &Row) -> &'static str {
    if r.endpoint.is_empty() || !r.has_secret {
        "UNKNOWN"
    } else if r.failing {
        "FAILING"
    } else {
        "WORKING"
    }
}

fn doc(r: &Row) -> Value {
    json!({
        "type": "archives",
        "id": r.id,
        "attributes": {
            "name": r.name,
            "query": dd_query(&r.query),
            "state": state(r),
            "destination": destination(r),
            "include_tags": r.include_tags,
            "rehydration_tags": r.rehydration_tags,
            "rehydration_max_scan_size_in_gb": r.rehydration_max_scan_size_gb,
        }
    })
}

async fn find(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    let Some(id) = uuid(id) else { return Err(not_found("Not found")) };
    sqlx::query_as(&format!("{SELECT} WHERE a.org_id = $1 AND a.id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?.ok_or_else(|| not_found("Not found"))
}

/// Where a Datadog destination is written: (endpoint, region, bucket, prefix, path style, access key).
fn target(d: &Value) -> Result<(String, String, String, String, bool, String), DdError> {
    let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let prefix = s("path").trim_matches('/').to_string();
    match s("type").as_str() {
        "s3" => {
            let bucket = s("bucket");
            if bucket.is_empty() {
                return Err(bad_request("An s3 destination needs a bucket"));
            }
            let key = d.get("integration").and_then(|i| i.get("access_key_id")).and_then(Value::as_str).unwrap_or("").to_string();
            Ok(("https://s3.amazonaws.com".into(), "us-east-1".into(), bucket, prefix, false, key))
        }
        "gcs" => {
            let bucket = s("bucket");
            if bucket.is_empty() {
                return Err(bad_request("A gcs destination needs a bucket"));
            }
            Ok(("https://storage.googleapis.com".into(), "auto".into(), bucket, prefix, true, String::new()))
        }
        "azure" => {
            let container = s("container");
            if container.is_empty() || s("storage_account").is_empty() {
                return Err(bad_request("An azure destination needs a storage account and a container"));
            }
            Ok((String::new(), String::new(), container, prefix, true, String::new()))
        }
        "" => Err(bad_request("The parameter 'destination' is required")),
        t => Err(bad_request(format!("Unknown destination type '{t}' (s3, gcs or azure)"))),
    }
}

struct Input {
    name: String,
    query: String,
    destination: Value,
    include_tags: bool,
    rehydration_tags: Vec<String>,
    max_scan_gb: Option<i64>,
}

fn input(raw: &Bytes) -> Result<Input, DdError> {
    let v = body(raw)?;
    let d = data(&v)?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != "archives") {
        return Err(bad_request("data.type must be archives"));
    }
    let a = d.get("attributes").cloned().unwrap_or(Value::Null);
    let name = a.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 100).ok_or_else(|| bad_request("The parameter 'name' is required"))?;
    let query = a.get("query").and_then(Value::as_str).ok_or_else(|| bad_request("The parameter 'query' is required"))?;
    let destination = a.get("destination").filter(|d| d.is_object()).cloned().ok_or_else(|| bad_request("The parameter 'destination' is required"))?;
    target(&destination)?;
    let max_scan_gb = match a.get("rehydration_max_scan_size_in_gb") {
        Some(Value::Number(n)) => Some(n.as_i64().filter(|n| *n > 0).ok_or_else(|| bad_request("rehydration_max_scan_size_in_gb must be a positive integer"))?),
        _ => None,
    };
    Ok(Input {
        name: name.to_string(),
        query: our_query(query),
        destination,
        include_tags: a.get("include_tags").and_then(Value::as_bool).unwrap_or(false),
        rehydration_tags: a.get("rehydration_tags").and_then(Value::as_array).map(|t| t.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        max_scan_gb,
    })
}

fn conflict(e: sqlx::Error, name: &str) -> DdError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => DdError::new(StatusCode::CONFLICT, format!("An archive named '{name}' already exists")),
        _ => e.into(),
    }
}

pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE a.org_id = $1 ORDER BY a.position, a.created_at")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    ok(json!({ "data": rows.iter().map(doc).collect::<Vec<_>>() }))
}

pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &id).await?) }))
}

pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let i = input(&raw)?;
    let (endpoint, region, bucket, prefix, path_style, key) = target(&i.destination)?;
    let hour = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64 / 3600 * 3600;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO log_archives (org_id, name, endpoint, region, bucket, prefix, path_style, access_key, secret_key_enc, query, start_hour, created_by, \
         destination, include_tags, rehydration_tags, rehydration_max_scan_size_gb, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '', $9, to_timestamp($10), $11::uuid, $12, $13, $14, $15, (SELECT COALESCE(max(position), -1) + 1 FROM log_archives WHERE org_id = $1)) \
         RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&i.name)
    .bind(&endpoint)
    .bind(&region)
    .bind(&bucket)
    .bind(&prefix)
    .bind(path_style)
    .bind(&key)
    .bind(&i.query)
    .bind(hour)
    .bind(&ctx.user_id)
    .bind(&i.destination)
    .bind(i.include_tags)
    .bind(serde_json::to_value(&i.rehydration_tags).unwrap_or_default())
    .bind(i.max_scan_gb)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| conflict(e, &i.name))?;
    ctx.audit(&st, "log_archive.created", &i.name, &format!("{bucket}/{prefix} (API)")).await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &id).await?) }))
}

/// Replaces the archive's definition. A changed bucket or account needs its keys set again.
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let current = find(&st, &ctx.org_id, &id).await?;
    let i = input(&raw)?;
    let (endpoint, region, bucket, prefix, path_style, key) = target(&i.destination)?;
    let moved = destination(&current).get("type") != i.destination.get("type") || bucket != current.bucket || (!key.is_empty() && key != current.access_key);
    sqlx::query(
        "UPDATE log_archives SET name = $3, query = $4, destination = $5, include_tags = $6, rehydration_tags = $7, rehydration_max_scan_size_gb = $8, prefix = $9, \
         endpoint = CASE WHEN $10 THEN $11 ELSE endpoint END, region = CASE WHEN $10 THEN $12 ELSE region END, bucket = $13, \
         path_style = CASE WHEN $10 THEN $14 ELSE path_style END, access_key = CASE WHEN $10 THEN $15 ELSE access_key END, \
         secret_key_enc = CASE WHEN $10 THEN '' ELSE secret_key_enc END, updated_at = now() WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&current.id)
    .bind(&i.name)
    .bind(&i.query)
    .bind(&i.destination)
    .bind(i.include_tags)
    .bind(serde_json::to_value(&i.rehydration_tags).unwrap_or_default())
    .bind(i.max_scan_gb)
    .bind(&prefix)
    .bind(moved)
    .bind(&endpoint)
    .bind(&region)
    .bind(&bucket)
    .bind(path_style)
    .bind(&key)
    .execute(&st.postgres)
    .await
    .map_err(|e| conflict(e, &i.name))?;
    ctx.audit(&st, "log_archive.updated", &i.name, "API").await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &current.id).await?) }))
}

/// Stops archiving; files already in the bucket stay there.
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let a = find(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_archives WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&a.id).execute(&st.postgres).await?;
    ctx.audit(&st, "log_archive.deleted", &a.name, "API").await;
    Ok(StatusCode::NO_CONTENT)
}

async fn order(st: &AppState, org: &str) -> Result<Vec<String>, DdError> {
    Ok(sqlx::query_scalar("SELECT id::text FROM log_archives WHERE org_id = $1 ORDER BY position, created_at").bind(org).fetch_all(&st.postgres).await?)
}

fn order_doc(ids: &[String]) -> Value {
    json!({ "data": { "type": "archive_order", "attributes": { "archive_ids": ids } } })
}

pub async fn get_order(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    ok(order_doc(&order(&st, &ctx.org_id).await?))
}

/// Every archive, once, in order.
pub async fn update_order(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let v = body(&raw)?;
    let ids: Vec<String> = data(&v)?
        .get("attributes")
        .and_then(|a| a.get("archive_ids"))
        .and_then(Value::as_array)
        .ok_or_else(|| bad_request("The parameter 'archive_ids' is required"))?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let current = order(&st, &ctx.org_id).await?;
    let (mut a, mut b) = (ids.clone(), current);
    a.sort();
    b.sort();
    if a != b {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "archive_ids must list every archive of the organization exactly once"));
    }
    let mut tx = st.postgres.begin().await?;
    for (pos, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE log_archives SET position = $3 WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(id).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ok(order_doc(&ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_map_to_buckets() {
        let (endpoint, _, bucket, prefix, path_style, key) = target(&json!({"type": "s3", "bucket": "logs-archive", "path": "/prod/", "integration": {"access_key_id": "AKIAEXAMPLE"}})).unwrap();
        assert_eq!((endpoint.as_str(), bucket.as_str(), prefix.as_str(), path_style, key.as_str()), ("https://s3.amazonaws.com", "logs-archive", "prod", false, "AKIAEXAMPLE"));
        assert!(target(&json!({"type": "gcs", "bucket": "b"})).unwrap().0.contains("googleapis"));
        assert!(target(&json!({"type": "azure", "container": "c", "storage_account": "acct"})).unwrap().0.is_empty());
        assert!(target(&json!({"type": "ftp"})).is_err());
        assert!(target(&json!({"type": "s3"})).is_err());
    }
}
