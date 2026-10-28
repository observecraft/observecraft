//! Generated metrics (Log Configuration → Generate Metrics): CRUD of the org's logs-based metric
//! definitions. The indexer (`logproc`) reloads them every 10 s and computes them on every
//! ingested log, before index routing; the engine is in `crate::log_metrics`.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_metrics::MAX_SERIES_PER_METRIC;
use crate::state::AppState;

const MAX_METRICS: i64 = 100;
const MAX_GROUP_BYS: usize = 10;
const MAX_NAME_CHARS: usize = 200;
const MAX_QUERY_CHARS: usize = 2_000;
/// Names the platform computes itself (`trace.*` metrics come from spans).
const RESERVED_PREFIXES: [&str; 3] = ["trace.", "datadog.", "observecraft."];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogMetricAggregation {
    /// Matching logs (a count metric).
    Count,
    /// Values of a numeric attribute (`path`), with percentiles.
    Distribution,
}

impl LogMetricAggregation {
    fn as_str(self) -> &'static str {
        match self {
            LogMetricAggregation::Count => "count",
            LogMetricAggregation::Distribution => "distribution",
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogMetricGroupBy {
    /// A reserved field (`service`, `status`, `host`, `source`), a tag (`env`) or an attribute (`@http.status_code`).
    pub path: String,
    /// The metric's tag; defaults to the path without `@`.
    #[serde(default)]
    pub tag_name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogMetric {
    pub id: String,
    pub name: String,
    /// Logs matching this search are counted; '' = every log.
    pub query: String,
    pub aggregation: LogMetricAggregation,
    /// The measured attribute of a distribution (`@duration`).
    pub path: Option<String>,
    pub group_by: Vec<LogMetricGroupBy>,
    /// Distinct group-by value sets counted per 10 s; logs of further series are dropped.
    pub max_series: u32,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogMetricInput {
    pub name: String,
    #[serde(default)]
    pub query: String,
    pub aggregation: LogMetricAggregation,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub group_by: Vec<LogMetricGroupBy>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    query: String,
    aggregation: String,
    path: Option<String>,
    group_by: Value,
}

impl From<Row> for LogMetric {
    fn from(r: Row) -> LogMetric {
        LogMetric {
            id: r.id,
            name: r.name,
            query: r.query,
            aggregation: if r.aggregation == "distribution" { LogMetricAggregation::Distribution } else { LogMetricAggregation::Count },
            path: r.path,
            group_by: serde_json::from_value(r.group_by).unwrap_or_default(),
            max_series: MAX_SERIES_PER_METRIC as u32,
        }
    }
}

/// `logs.checkout.errors`: starts with a letter; letters, digits, `_` and `.`.
fn valid_metric_name(n: &str) -> bool {
    n.starts_with(|c: char| c.is_ascii_alphabetic()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !n.ends_with('.') && !n.contains("..")
}

/// A group-by or measure path: `service`, `env`, `@http.status_code`.
fn valid_path(p: &str) -> bool {
    let key = p.strip_prefix('@').unwrap_or(p);
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// A tag name metric queries accept (`by {http.status_code}`).
fn valid_tag(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')) && !t.starts_with("oc.")
}

/// Trims and checks a definition.
fn validate(input: &mut LogMetricInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    input.query = input.query.trim().to_string();
    let name = &input.name;
    if name.len() > MAX_NAME_CHARS || !valid_metric_name(name) {
        return Err(format!("Metric names are up to {MAX_NAME_CHARS} letters, digits, underscores and dots, starting with a letter (e.g. logs.checkout.errors)"));
    }
    if let Some(prefix) = RESERVED_PREFIXES.iter().find(|p| name.starts_with(*p)) {
        return Err(format!("Metric names can't start with '{prefix}'"));
    }
    if input.query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("The query is at most {MAX_QUERY_CHARS} characters"));
    }
    input.path = match input.aggregation {
        LogMetricAggregation::Count => None,
        LogMetricAggregation::Distribution => match input.path.as_deref().map(str::trim) {
            Some(p) if p.starts_with('@') && valid_path(p) => Some(p.to_string()),
            _ => return Err("A distribution measures a numeric attribute, e.g. @duration".into()),
        },
    };
    if input.group_by.len() > MAX_GROUP_BYS {
        return Err(format!("At most {MAX_GROUP_BYS} group-bys"));
    }
    let mut tags = HashSet::new();
    for g in &mut input.group_by {
        g.path = g.path.trim().to_string();
        if !valid_path(&g.path) {
            return Err(format!("Can't group by '{}': use a tag (env), a reserved field (service) or an attribute (@http.status_code)", g.path));
        }
        g.tag_name = match g.tag_name.trim() {
            "" => g.path.trim_start_matches('@').to_string(),
            t => t.to_string(),
        };
        if !valid_tag(&g.tag_name) {
            return Err(format!("'{}' isn't a valid tag name", g.tag_name));
        }
        if !tags.insert(g.tag_name.clone()) {
            return Err(format!("Two group-bys are tagged '{}'", g.tag_name));
        }
    }
    Ok(())
}

const COLS: &str = "id::text AS id, name, query, aggregation, path, group_by";

async fn get_metric(st: &AppState, org: &str, id: &str) -> Result<LogMetric, ApiError> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM log_metrics WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(LogMetric::from).ok_or_else(|| ApiError::NotFound("Generated metric not found".into()))
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A metric named '{name}' already exists")),
        _ => e.into(),
    }
}

fn group_by_json(g: &[LogMetricGroupBy]) -> Value {
    serde_json::to_value(g).unwrap_or(Value::Array(vec![]))
}

#[utoipa::path(get, path = "/api/v1/logs/metrics", operation_id = "list_log_metrics", responses((status = 200, body = Vec<LogMetric>)), tag = "logs")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<LogMetric>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM log_metrics WHERE org_id = $1 ORDER BY name")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(LogMetric::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/logs/metrics", operation_id = "create_log_metric", request_body = LogMetricInput, responses((status = 200, body = LogMetric)), tag = "logs")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<LogMetricInput>) -> ApiResult<LogMetric> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_metrics WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_METRICS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_METRICS} generated metrics")));
    }
    let (id,): (String,) = sqlx::query_as("INSERT INTO log_metrics (org_id, name, query, aggregation, path, group_by) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&input.name)
        .bind(&input.query)
        .bind(input.aggregation.as_str())
        .bind(&input.path)
        .bind(group_by_json(&input.group_by))
        .fetch_one(&st.postgres)
        .await
        .map_err(|e| unique(e, &input.name))?;
    ctx.audit(&st, "log_metric.created", &input.name, &input.query).await;
    Ok(Json(get_metric(&st, &ctx.org_id, &id).await?))
}

/// Changes the query, aggregation or group-bys; the name stays.
#[utoipa::path(put, path = "/api/v1/logs/metrics/{id}", operation_id = "update_log_metric", params(("id" = String, Path)), request_body = LogMetricInput, responses((status = 200, body = LogMetric)), tag = "logs")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<LogMetricInput>) -> ApiResult<LogMetric> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let current = get_metric(&st, &ctx.org_id, &id).await?;
    if current.name != input.name {
        return Err(ApiError::BadRequest("A generated metric can't be renamed: create a new one".into()));
    }
    sqlx::query("UPDATE log_metrics SET query = $3, aggregation = $4, path = $5, group_by = $6, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.query)
        .bind(input.aggregation.as_str())
        .bind(&input.path)
        .bind(group_by_json(&input.group_by))
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "log_metric.updated", &input.name, &input.query).await;
    Ok(Json(get_metric(&st, &ctx.org_id, &id).await?))
}

/// Stops computing it; points already written stay until they expire.
#[utoipa::path(delete, path = "/api/v1/logs/metrics/{id}", operation_id = "delete_log_metric", params(("id" = String, Path)), responses((status = 204)), tag = "logs")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM log_metrics WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Generated metric not found".into())) };
    ctx.audit(&st, "log_metric.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, aggregation: LogMetricAggregation, path: Option<&str>, group_by: &[&str]) -> LogMetricInput {
        LogMetricInput {
            name: name.into(),
            query: " service:order-service status:error ".into(),
            aggregation,
            path: path.map(str::to_string),
            group_by: group_by.iter().map(|p| LogMetricGroupBy { path: p.to_string(), tag_name: String::new() }).collect(),
        }
    }

    use LogMetricAggregation::{Count, Distribution};

    #[test]
    fn names_are_metric_names() {
        let mut ok = input("  logs.checkout.errors ", Count, None, &[]);
        assert_eq!(validate(&mut ok), Ok(()));
        assert_eq!((ok.name.as_str(), ok.query.as_str()), ("logs.checkout.errors", "service:order-service status:error"));
        for bad in ["", "1logs", "logs checkout", "logs.", "logs..x", "logs-x", "@logs", "trace.http.hits", "datadog.x"] {
            assert!(validate(&mut input(bad, Count, None, &[])).is_err(), "{bad:?}");
        }
        assert!(validate(&mut input(&"x".repeat(MAX_NAME_CHARS + 1), Count, None, &[])).is_err());
        assert!(validate(&mut input("logs.Mixed_Case.v2", Count, None, &[])).is_ok());
    }

    #[test]
    fn distributions_measure_an_attribute() {
        let mut d = input("logs.checkout.latency", Distribution, Some(" @duration "), &[]);
        assert_eq!(validate(&mut d), Ok(()));
        assert_eq!(d.path.as_deref(), Some("@duration"));
        assert!(validate(&mut input("logs.x", Distribution, None, &[])).is_err());
        assert!(validate(&mut input("logs.x", Distribution, Some("duration"), &[])).is_err(), "a measure is an attribute");
        assert!(validate(&mut input("logs.x", Distribution, Some("@bad path"), &[])).is_err());
        // A count has no measure.
        let mut c = input("logs.x", Count, Some("@duration"), &[]);
        assert_eq!(validate(&mut c), Ok(()));
        assert_eq!(c.path, None);
    }

    #[test]
    fn group_bys_become_tags() {
        let mut g = input("logs.x", Count, None, &["service", " @http.status_code ", "kube_namespace"]);
        g.group_by.push(LogMetricGroupBy { path: "@usr.id".into(), tag_name: " user ".into() });
        assert_eq!(validate(&mut g), Ok(()));
        let tags: Vec<_> = g.group_by.iter().map(|x| (x.path.as_str(), x.tag_name.as_str())).collect();
        assert_eq!(tags, vec![("service", "service"), ("@http.status_code", "http.status_code"), ("kube_namespace", "kube_namespace"), ("@usr.id", "user")]);
        let many: Vec<String> = (0..=MAX_GROUP_BYS).map(|i| format!("@a{i}")).collect();
        assert!(validate(&mut input("logs.x", Count, None, &many.iter().map(String::as_str).collect::<Vec<_>>())).is_err());
        assert!(validate(&mut input("logs.x", Count, None, &["service", "service"])).is_err(), "tags are unique");
        assert!(validate(&mut input("logs.x", Count, None, &["@a b"])).is_err());
        assert!(validate(&mut input("logs.x", Count, None, &["@"])).is_err());
        assert!(validate(&mut input("logs.x", Count, None, &["@oc.org_id"])).is_err(), "oc.* tags are the platform's");
    }

    #[test]
    fn queries_are_bounded() {
        let mut long = input("logs.x", Count, None, &[]);
        long.query = "a".repeat(MAX_QUERY_CHARS + 1);
        assert!(validate(&mut long).is_err());
    }
}
