//! Log indexes, Datadog-compatible (`/api/v1/logs/config/indexes`, `/index-order`): the provider's
//! `datadog_logs_index` and `datadog_logs_index_order`, on `log_indexes` ([`crate::routes::log_indexes`]).
//!
//! - An index is addressed by its name; names are lowercase here (Datadog's are too in practice).
//! - `num_retention_days` is the standard tier; `num_flex_logs_retention_days` the total time with the
//!   flex tier, applied while the org has Flex Logs on (Logs → Configuration → Flex Logs Controls).
//! - `daily_limit_reset` is kept as written; the indexer resets quotas at the matching UTC hour.
//! - Exclusion filters' `sample_rate` is a fraction (0–1); we store percents.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::super::users::{bad_request, body, not_found, ok, require, DocResult};
use super::super::{DdAuth, DdError};
use super::{dd_query, our_query};
use crate::auth::Access;
use crate::error::ApiError;
use crate::routes::log_indexes::{data_settings, validate, ExclusionFilter, LogIndexInput};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct Row {
    name: String,
    filter: String,
    retention_days: i32,
    daily_quota: Option<i64>,
    quota_reset_hour: i16,
    exclusion_filters: Value,
    flex_retention_days: Option<i32>,
    tags: Value,
    daily_limit_warning_threshold: Option<f64>,
    reset_utc_offset: String,
}

const COLS: &str = "name, filter, retention_days, daily_quota, quota_reset_hour, exclusion_filters, flex_retention_days, tags, daily_limit_warning_threshold, reset_utc_offset";

/// `+02:00` → 2 (hours east of UTC).
fn offset_hours(offset: &str) -> Option<i16> {
    let (sign, rest) = match offset.as_bytes().first()? {
        b'+' => (1, &offset[1..]),
        b'-' => (-1, &offset[1..]),
        _ => return None,
    };
    let (h, m) = rest.split_once(':')?;
    if m != "00" || h.len() != 2 {
        return None;
    }
    let h: i16 = h.parse().ok()?;
    (-12..=14).contains(&(sign * h)).then_some(sign * h)
}

/// `HH:00` → HH.
fn reset_hour(time: &str) -> Option<i16> {
    let (h, m) = time.split_once(':')?;
    let h: i16 = h.parse().ok()?;
    (m == "00" && (0..=23).contains(&h) && h.to_string().len() <= 2).then_some(h)
}

fn doc(r: &Row, rate_limited: bool) -> Value {
    let offset = offset_hours(&r.reset_utc_offset).unwrap_or(0);
    let local = (r.quota_reset_hour + offset).rem_euclid(24);
    let filters: Vec<ExclusionFilter> = serde_json::from_value(r.exclusion_filters.clone()).unwrap_or_default();
    let mut d = json!({
        "name": r.name,
        "filter": { "query": dd_query(&r.filter) },
        "num_retention_days": r.retention_days,
        "daily_limit": r.daily_quota,
        "daily_limit_reset": { "reset_time": format!("{local:02}:00"), "reset_utc_offset": r.reset_utc_offset },
        "daily_limit_warning_threshold_percentage": r.daily_limit_warning_threshold,
        "is_rate_limited": rate_limited,
        "exclusion_filters": filters.iter().map(|e| {
            let mut f = json!({ "query": dd_query(&e.query), "sample_rate": e.sample_rate / 100.0 });
            if let Some(a) = &e.sample_attribute {
                f["sample_attribute"] = Value::from(a.clone());
            }
            json!({ "name": e.name, "is_enabled": e.enabled, "filter": f })
        }).collect::<Vec<_>>(),
        "tags": r.tags,
    });
    if let Some(flex) = r.flex_retention_days {
        d["num_flex_logs_retention_days"] = Value::from(flex);
    }
    d
}

async fn find(st: &AppState, org: &str, name: &str) -> Result<Row, DdError> {
    sqlx::query_as(&format!("SELECT {COLS} FROM log_indexes WHERE org_id = $1 AND name = $2"))
        .bind(org)
        .bind(name)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| not_found(format!("Index {name} not found")))
}

/// The writable fields of an index document, over `current` when updating.
struct Input {
    index: LogIndexInput,
    warning: Option<f64>,
    offset: String,
}

fn input(v: &Value, current: Option<&Row>) -> Result<Input, DdError> {
    if !v.is_object() {
        return Err(bad_request("The body must be an index object"));
    }
    let name = match current {
        Some(c) => c.name.clone(),
        None => v.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| bad_request("The parameter 'name' is required"))?.to_string(),
    };
    let filter = match v.get("filter") {
        Some(f) => our_query(f.get("query").and_then(Value::as_str).unwrap_or("")),
        None => current.map(|c| c.filter.clone()).ok_or_else(|| bad_request("The parameter 'filter' is required"))?,
    };
    let retention_days = match v.get("num_retention_days") {
        Some(Value::Number(n)) => n.as_i64().ok_or_else(|| bad_request("num_retention_days must be an integer"))? as i32,
        _ => current.map_or(15, |c| c.retention_days),
    };
    let disable = v.get("disable_daily_limit").and_then(Value::as_bool).unwrap_or(false);
    let daily_quota = if disable {
        None
    } else {
        match v.get("daily_limit") {
            Some(Value::Number(n)) => Some(n.as_i64().filter(|n| *n > 0).ok_or_else(|| bad_request("daily_limit must be a positive integer"))?),
            Some(Value::Null) => None,
            Some(_) => return Err(bad_request("daily_limit must be an integer")),
            None => current.and_then(|c| c.daily_quota),
        }
    };
    let (mut hour, mut offset) = current.map_or((0, "+00:00".to_string()), |c| (c.quota_reset_hour, c.reset_utc_offset.clone()));
    if let Some(reset) = v.get("daily_limit_reset").filter(|r| r.is_object()) {
        let time = reset.get("reset_time").and_then(Value::as_str).unwrap_or("00:00");
        let off = reset.get("reset_utc_offset").and_then(Value::as_str).unwrap_or("+00:00");
        let local = reset_hour(time).ok_or_else(|| bad_request("reset_time is HH:00"))?;
        let east = offset_hours(off).ok_or_else(|| bad_request("reset_utc_offset is (+|-)HH:00, from -12:00 to +14:00"))?;
        hour = (local - east).rem_euclid(24);
        offset = off.to_string();
    }
    let warning = match v.get("daily_limit_warning_threshold_percentage") {
        Some(Value::Number(n)) => {
            let p = n.as_f64().unwrap_or(0.0);
            if !(50.0..=99.99).contains(&p) {
                return Err(bad_request("daily_limit_warning_threshold_percentage is between 50 and 99.99"));
            }
            Some(p)
        }
        Some(Value::Null) => None,
        Some(_) => return Err(bad_request("daily_limit_warning_threshold_percentage must be a number")),
        None => current.and_then(|c| c.daily_limit_warning_threshold),
    };
    let exclusion_filters = match v.get("exclusion_filters") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|e| {
                let f = e.get("filter").cloned().unwrap_or(Value::Null);
                let rate = f.get("sample_rate").and_then(Value::as_f64).unwrap_or(1.0);
                if !(0.0..=1.0).contains(&rate) {
                    return Err(bad_request("An exclusion filter's sample_rate is between 0 and 1"));
                }
                Ok(ExclusionFilter {
                    name: e.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                    query: our_query(f.get("query").and_then(Value::as_str).unwrap_or("")),
                    sample_rate: rate * 100.0,
                    enabled: e.get("is_enabled").and_then(Value::as_bool).unwrap_or(false),
                    sample_attribute: f.get("sample_attribute").and_then(Value::as_str).filter(|a| !a.is_empty()).map(str::to_string),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::Null) | None => current.map(|c| serde_json::from_value(c.exclusion_filters.clone()).unwrap_or_default()).unwrap_or_default(),
        Some(_) => return Err(bad_request("exclusion_filters must be a list")),
    };
    let flex_retention_days = match v.get("num_flex_logs_retention_days") {
        Some(Value::Number(n)) => n.as_i64().map(|n| n as i32),
        Some(Value::Null) => None,
        Some(_) => return Err(bad_request("num_flex_logs_retention_days must be an integer")),
        None => current.and_then(|c| c.flex_retention_days),
    };
    let tags = match v.get("tags") {
        Some(Value::Array(t)) => t.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::Null) => vec![],
        Some(_) => return Err(bad_request("tags must be a list")),
        None => current.map(|c| serde_json::from_value(c.tags.clone()).unwrap_or_default()).unwrap_or_default(),
    };
    Ok(Input {
        index: LogIndexInput { name, filter, retention_days, daily_quota, quota_reset_hour: hour, exclusion_filters, flex_retention_days, tags },
        warning,
        offset,
    })
}

async fn checked(st: &AppState, org: &str, mut i: Input) -> Result<Input, DdError> {
    let settings = data_settings(st, org).await?;
    validate(&mut i.index, settings.max_log_retention_days).map_err(DdError::from)?;
    Ok(i)
}

/// The index reached its daily quota (counted since the current quota day started).
async fn rate_limited(st: &AppState, org: &str, r: &Row) -> bool {
    let Some(quota) = r.daily_quota else { return false };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let offset = r.quota_reset_hour as i64 * 3_600_000;
    let start = (now - offset).div_euclid(86_400_000) * 86_400_000 + offset;
    let n = st
        .ch(org)
        .query("SELECT count() FROM logs WHERE org_id = ? AND index_name = ? AND timestamp >= fromUnixTimestamp64Milli(?)")
        .bind(org)
        .bind(&r.name)
        .bind(start)
        .fetch_one::<u64>()
        .await
        .unwrap_or(0);
    n as i64 >= quota
}

pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM log_indexes WHERE org_id = $1 ORDER BY position, created_at")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let mut docs = Vec::with_capacity(rows.len());
    for r in &rows {
        docs.push(doc(r, rate_limited(&st, &ctx.org_id, r).await));
    }
    ok(json!({ "indexes": docs }))
}

pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(name): Path<String>) -> DocResult {
    let r = find(&st, &ctx.org_id, &name).await?;
    ok(doc(&r, rate_limited(&st, &ctx.org_id, &r).await))
}

pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let i = checked(&st, &ctx.org_id, input(&body(&raw)?, None)?).await?;
    let settings = data_settings(&st, &ctx.org_id).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_indexes WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= settings.max_indexes as i64 {
        return Err(bad_request(format!("This organization can have at most {} indexes", settings.max_indexes)));
    }
    let x = &i.index;
    let res = sqlx::query(
        "INSERT INTO log_indexes (org_id, name, filter, retention_days, daily_quota, quota_reset_hour, exclusion_filters, flex_retention_days, tags, \
         daily_limit_warning_threshold, reset_utc_offset, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, (SELECT COALESCE(max(position), -1) + 1 FROM log_indexes WHERE org_id = $1))",
    )
    .bind(&ctx.org_id)
    .bind(&x.name)
    .bind(&x.filter)
    .bind(x.retention_days)
    .bind(x.daily_quota)
    .bind(x.quota_reset_hour)
    .bind(serde_json::to_value(&x.exclusion_filters).unwrap_or_default())
    .bind(x.flex_retention_days)
    .bind(serde_json::to_value(&x.tags).unwrap_or_default())
    .bind(i.warning)
    .bind(&i.offset)
    .execute(&st.postgres)
    .await;
    match res {
        Err(sqlx::Error::Database(d)) if d.is_unique_violation() => return Err(DdError::new(StatusCode::CONFLICT, format!("Index {} already exists", x.name))),
        r => r?,
    };
    ctx.audit(&st, "log_index.created", &x.name, &format!("{} days (API)", x.retention_days)).await;
    let r = find(&st, &ctx.org_id, &x.name).await?;
    ok(doc(&r, false))
}

pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(name): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let current = find(&st, &ctx.org_id, &name).await?;
    let i = checked(&st, &ctx.org_id, input(&body(&raw)?, Some(&current))?).await?;
    let x = &i.index;
    sqlx::query(
        "UPDATE log_indexes SET filter = $3, retention_days = $4, daily_quota = $5, quota_reset_hour = $6, exclusion_filters = $7, flex_retention_days = $8, tags = $9, \
         daily_limit_warning_threshold = $10, reset_utc_offset = $11, updated_at = now() WHERE org_id = $1 AND name = $2",
    )
    .bind(&ctx.org_id)
    .bind(&name)
    .bind(&x.filter)
    .bind(x.retention_days)
    .bind(x.daily_quota)
    .bind(x.quota_reset_hour)
    .bind(serde_json::to_value(&x.exclusion_filters).unwrap_or_default())
    .bind(x.flex_retention_days)
    .bind(serde_json::to_value(&x.tags).unwrap_or_default())
    .bind(i.warning)
    .bind(&i.offset)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "log_index.updated", &name, &format!("{} days, quota {:?} (API)", x.retention_days, x.daily_quota)).await;
    let r = find(&st, &ctx.org_id, &name).await?;
    ok(doc(&r, rate_limited(&st, &ctx.org_id, &r).await))
}

/// Stops indexing into it; its logs stay searchable until they expire.
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(name): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let gone = sqlx::query("DELETE FROM log_indexes WHERE org_id = $1 AND name = $2").bind(&ctx.org_id).bind(&name).execute(&st.postgres).await?.rows_affected();
    if gone == 0 {
        return Err(not_found(format!("Index {name} not found")));
    }
    ctx.audit(&st, "log_index.deleted", &name, "API").await;
    ok(json!({}))
}

async fn order(st: &AppState, org: &str) -> Result<Vec<String>, DdError> {
    Ok(sqlx::query_scalar("SELECT name FROM log_indexes WHERE org_id = $1 ORDER BY position, created_at").bind(org).fetch_all(&st.postgres).await?)
}

pub async fn get_order(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    ok(json!({ "index_names": order(&st, &ctx.org_id).await? }))
}

/// Every index, once, in routing order.
pub async fn update_order(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let v = body(&raw)?;
    let names: Vec<String> = v.get("index_names").and_then(Value::as_array).ok_or_else(|| bad_request("The parameter 'index_names' is required"))?.iter().filter_map(Value::as_str).map(str::to_string).collect();
    let current = order(&st, &ctx.org_id).await?;
    let (mut a, mut b) = (names.clone(), current.clone());
    a.sort();
    b.sort();
    if a != b {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "index_names must list every index of the organization exactly once"));
    }
    let mut tx = st.postgres.begin().await.map_err(ApiError::from)?;
    for (pos, name) in names.iter().enumerate() {
        sqlx::query("UPDATE log_indexes SET position = $3 WHERE org_id = $1 AND name = $2").bind(&ctx.org_id).bind(name).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "log_index.reordered", "", "API").await;
    ok(json!({ "index_names": names }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_times_and_offsets() {
        assert_eq!(offset_hours("+02:00"), Some(2));
        assert_eq!(offset_hours("-05:00"), Some(-5));
        assert_eq!(offset_hours("+14:00"), Some(14));
        assert_eq!(offset_hours("+15:00"), None);
        assert_eq!(offset_hours("+05:30"), None);
        assert_eq!(reset_hour("14:00"), Some(14));
        assert_eq!(reset_hour("24:00"), None);
        assert_eq!(reset_hour("14:30"), None);
    }

    #[test]
    fn documents_map_onto_our_fields() {
        let v = json!({
            "name": "web", "filter": {"query": "*"}, "num_retention_days": 7, "daily_limit": 1000,
            "daily_limit_reset": {"reset_time": "14:00", "reset_utc_offset": "+02:00"},
            "exclusion_filters": [{"name": "debug", "is_enabled": true, "filter": {"query": "status:debug", "sample_rate": 0.5}}],
            "num_flex_logs_retention_days": 90, "tags": ["team:web"]
        });
        let i = input(&v, None).unwrap();
        assert_eq!(i.index.filter, "");
        assert_eq!(i.index.quota_reset_hour, 12);
        assert_eq!(i.index.exclusion_filters[0].sample_rate, 50.0);
        assert_eq!(i.index.flex_retention_days, Some(90));
        let row = Row {
            name: "web".into(),
            filter: String::new(),
            retention_days: 7,
            daily_quota: Some(1000),
            quota_reset_hour: 12,
            exclusion_filters: serde_json::to_value(&i.index.exclusion_filters).unwrap(),
            flex_retention_days: Some(90),
            tags: json!(["team:web"]),
            daily_limit_warning_threshold: None,
            reset_utc_offset: "+02:00".into(),
        };
        let d = doc(&row, false);
        assert_eq!(d["daily_limit_reset"]["reset_time"], "14:00");
        assert_eq!(d["exclusion_filters"][0]["filter"]["sample_rate"], 0.5);
        assert_eq!(d["filter"]["query"], "*");
        assert!(input(&json!({"filter": {"query": "*"}}), None).is_err());
    }
}
