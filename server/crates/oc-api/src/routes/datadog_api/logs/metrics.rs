//! Log-based metrics, Datadog-compatible (`/api/v2/logs/config/metrics`): the provider's
//! `datadog_logs_metric`, on `log_metrics` ([`crate::routes::log_metrics`]).
//!
//! - A metric's id is its name. Characters metric names can't hold become `_`, as Datadog does
//!   (`checkout-errors` → `checkout_errors`).
//! - The aggregation type can't change after creation; filter, group-bys and `include_percentiles` can.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::super::users::{bad_request, body, data, not_found, ok, require, DocResult};
use super::super::{DdAuth, DdError};
use super::{dd_query, our_query};
use crate::auth::Access;
use crate::routes::log_metrics::{validate, LogMetricAggregation, LogMetricGroupBy, LogMetricInput};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct Row {
    name: String,
    query: String,
    aggregation: String,
    path: Option<String>,
    group_by: Value,
    include_percentiles: bool,
}

const SELECT: &str = "SELECT name, query, aggregation, path, group_by, include_percentiles FROM log_metrics";

fn doc(r: &Row) -> Value {
    let mut compute = json!({ "aggregation_type": r.aggregation });
    if let Some(p) = &r.path {
        compute["path"] = Value::from(p.clone());
        compute["include_percentiles"] = Value::Bool(r.include_percentiles);
    }
    let group_by: Vec<LogMetricGroupBy> = serde_json::from_value(r.group_by.clone()).unwrap_or_default();
    json!({
        "type": "logs_metrics",
        "id": r.name,
        "attributes": {
            "compute": compute,
            "filter": { "query": dd_query(&r.query) },
            "group_by": group_by.iter().map(|g| json!({ "path": g.path, "tag_name": g.tag_name })).collect::<Vec<_>>(),
        }
    })
}

/// Datadog's normalization of metric names.
fn metric_name(id: &str) -> String {
    id.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).collect()
}

async fn find(st: &AppState, org: &str, name: &str) -> Result<Row, DdError> {
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND name = $2"))
        .bind(org)
        .bind(name)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| not_found(format!("Metric {name} not found")))
}

fn group_by(a: &Value) -> Result<Option<Vec<LogMetricGroupBy>>, DdError> {
    match a.get("group_by") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => Ok(Some(
            items
                .iter()
                .map(|g| LogMetricGroupBy {
                    path: g.get("path").and_then(Value::as_str).unwrap_or("").to_string(),
                    tag_name: g.get("tag_name").and_then(Value::as_str).unwrap_or("").to_string(),
                })
                .collect(),
        )),
        Some(_) => Err(bad_request("group_by must be a list")),
    }
}

fn checked(mut i: LogMetricInput) -> Result<LogMetricInput, DdError> {
    validate(&mut i).map_err(bad_request)?;
    Ok(i)
}

pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 ORDER BY name")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    ok(json!({ "data": rows.iter().map(doc).collect::<Vec<_>>() }))
}

pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &id).await?) }))
}

pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let v = body(&raw)?;
    let d = data(&v)?;
    let id = d.get("id").and_then(Value::as_str).filter(|i| !i.trim().is_empty()).ok_or_else(|| bad_request("The parameter 'id' is required"))?;
    let a = d.get("attributes").cloned().unwrap_or(Value::Null);
    let compute = a.get("compute").ok_or_else(|| bad_request("The parameter 'compute' is required"))?;
    let aggregation = match compute.get("aggregation_type").and_then(Value::as_str) {
        Some("count") => LogMetricAggregation::Count,
        Some("distribution") => LogMetricAggregation::Distribution,
        _ => return Err(bad_request("compute.aggregation_type must be count or distribution")),
    };
    let i = checked(LogMetricInput {
        name: metric_name(id),
        query: our_query(a.get("filter").and_then(|f| f.get("query")).and_then(Value::as_str).unwrap_or("")),
        aggregation,
        path: compute.get("path").and_then(Value::as_str).map(str::to_string),
        group_by: group_by(&a)?.unwrap_or_default(),
        include_percentiles: compute.get("include_percentiles").and_then(Value::as_bool).unwrap_or(false),
    })?;
    let res = sqlx::query("INSERT INTO log_metrics (org_id, name, query, aggregation, path, group_by, include_percentiles) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&ctx.org_id)
        .bind(&i.name)
        .bind(&i.query)
        .bind(i.aggregation.as_str())
        .bind(&i.path)
        .bind(serde_json::to_value(&i.group_by).unwrap_or_default())
        .bind(i.include_percentiles && i.aggregation == LogMetricAggregation::Distribution)
        .execute(&st.postgres)
        .await;
    match res {
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(DdError::new(StatusCode::CONFLICT, format!("Metric {} already exists", i.name))),
        r => r?,
    };
    ctx.audit(&st, "log_metric.created", &i.name, "API").await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &i.name).await?) }))
}

pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let current = find(&st, &ctx.org_id, &id).await?;
    let v = body(&raw)?;
    let a = data(&v)?.get("attributes").cloned().unwrap_or(Value::Null);
    let compute = a.get("compute");
    let aggregation = if current.aggregation == "distribution" { LogMetricAggregation::Distribution } else { LogMetricAggregation::Count };
    let i = checked(LogMetricInput {
        name: current.name.clone(),
        query: match a.get("filter").and_then(|f| f.get("query")).and_then(Value::as_str) {
            Some(q) => our_query(q),
            None => current.query.clone(),
        },
        aggregation,
        path: current.path.clone(),
        group_by: match group_by(&a)? {
            Some(g) => g,
            None => serde_json::from_value(current.group_by.clone()).unwrap_or_default(),
        },
        include_percentiles: compute.and_then(|c| c.get("include_percentiles")).and_then(Value::as_bool).unwrap_or(current.include_percentiles),
    })?;
    sqlx::query("UPDATE log_metrics SET query = $3, group_by = $4, include_percentiles = $5, updated_at = now() WHERE org_id = $1 AND name = $2")
        .bind(&ctx.org_id)
        .bind(&i.name)
        .bind(&i.query)
        .bind(serde_json::to_value(&i.group_by).unwrap_or_default())
        .bind(i.include_percentiles && i.aggregation == LogMetricAggregation::Distribution)
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "log_metric.updated", &i.name, "API").await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &i.name).await?) }))
}

pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    let gone = sqlx::query("DELETE FROM log_metrics WHERE org_id = $1 AND name = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?.rows_affected();
    if gone == 0 {
        return Err(not_found(format!("Metric {id} not found")));
    }
    ctx.audit(&st, "log_metric.deleted", &id, "API").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized_like_datadog() {
        assert_eq!(metric_name("Test-Update_a_log_based_metric-1686138962"), "Test_Update_a_log_based_metric_1686138962");
        assert_eq!(metric_name("logs.checkout.errors"), "logs.checkout.errors");
    }
}
