//! Derived metrics (Metrics → Derived Metrics): a metric name that queries resolve to a formula over
//! existing metric queries, evaluated at query time. `avg:checkout.error_ratio{env:prod} by {host}`
//! with `checkout.error_ratio` = `sum:a{*} by {host} / sum:b{*} by {host} * 100` evaluates
//! `sum:a{env:prod} by {host} / sum:b{env:prod} by {host} * 100`.
//!
//! Resolution ([`expand`]) is textual, before a query or formula is parsed, so the explorer,
//! dashboards, monitors and SLOs all see derived names:
//! - the derived query's filters are added to every query of its formula, and its `by` (when it
//!   has one) replaces theirs;
//! - its space aggregator is not applied: each query of the formula aggregates as written;
//! - limits: a formula can't use another derived metric (no nesting), and `.rollup()` isn't
//!   supported on a derived name.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::parse_metric_query;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::monitor_formula::parse_formula;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_DERIVED: i64 = 200;
const MAX_QUERIES: usize = 10;
const MAX_NAME_CHARS: usize = 200;
const MAX_QUERY_CHARS: usize = 2_000;
const LETTERS: &str = "abcdefghij";
/// Names the platform computes itself.
const RESERVED_PREFIXES: [&str; 3] = ["trace.", "datadog.", "observecraft."];
/// How long an org's derived metrics are used before they are reloaded (other servers' changes).
const TTL: Duration = Duration::from_secs(30);

/// Derived name → formula.
pub(crate) type Derived = HashMap<String, String>;

/// The expression with each query letter replaced by its query in parentheses (`a / b * 100` →
/// `(sum:x{*}) / (sum:y{*}) * 100`), checked to be a valid formula.
pub(crate) fn compile(queries: &[String], expression: &str) -> Result<String, String> {
    if queries.is_empty() || queries.len() > MAX_QUERIES {
        return Err(format!("A derived metric has 1 to {MAX_QUERIES} queries"));
    }
    for (i, q) in queries.iter().enumerate() {
        parse_metric_query(q).map_err(|e| format!("Query {}: {e}", letter(i)))?;
    }
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("The formula is required, e.g. a / b * 100".into());
    }
    let mut out = String::new();
    let mut chars = expression.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() {
            let mut word = c.to_string();
            while let Some(&n) = chars.peek().filter(|n| n.is_ascii_alphanumeric() || **n == '_') {
                word.push(n);
                chars.next();
            }
            let index = (word.len() == 1).then(|| LETTERS.find(word.to_ascii_lowercase().as_str())).flatten();
            match index.and_then(|i| queries.get(i)) {
                Some(q) => out.push_str(&format!("({})", q.trim())),
                None if word == "default_zero" => out.push_str(&word),
                None => return Err(format!("'{word}' isn't a query letter of this metric")),
            }
        } else if c.is_ascii_digit() || " .+-*/()".contains(c) {
            out.push(c);
        } else {
            return Err(format!("Unexpected '{c}': use the query letters, numbers, + - * / and parentheses"));
        }
    }
    parse_formula(&out).map_err(|e| format!("Invalid formula: {e}"))?;
    Ok(out)
}

fn letter(i: usize) -> char {
    LETTERS.as_bytes().get(i).map_or('?', |b| *b as char)
}

/// The query or formula `text` with every derived metric query replaced by its formula (in
/// parentheses), merged with that query's filters and group-by. Unchanged when it names none.
pub(crate) fn expand(text: &str, derived: &Derived) -> Result<String, String> {
    if derived.is_empty() {
        return Ok(text.to_string());
    }
    let f = parse_formula(text)?;
    let trimmed = text.trim();
    let mut out = trimmed.to_string();
    let mut changed = false;
    // From the end, so earlier spans stay valid.
    for (q, &(start, end)) in f.queries.iter().zip(&f.spans).rev() {
        let outer = parse_metric_query(q)?;
        let Some(formula) = derived.get(&outer.metric) else { continue };
        if outer.rollup.is_some() {
            return Err(format!("'{}' is a derived metric: .rollup() isn't supported on it", outer.metric));
        }
        let inner = parse_formula(formula)?;
        let mut body = formula.trim().to_string();
        for (iq, &(is, ie)) in inner.queries.iter().zip(&inner.spans).rev() {
            let mut merged = parse_metric_query(iq)?;
            if derived.contains_key(&merged.metric) {
                return Err(format!("'{}' uses the derived metric '{}': derived metrics can't be nested", outer.metric, merged.metric));
            }
            for filter in &outer.filters {
                if !merged.filters.contains(filter) {
                    merged.filters.push(filter.clone());
                }
            }
            if !outer.by.is_empty() {
                merged.by = outer.by.clone();
            }
            body.replace_range(is..ie, &merged.to_string());
        }
        out.replace_range(start..end, &format!("({body})"));
        changed = true;
    }
    Ok(if changed { out } else { text.to_string() })
}

/// Org → its derived metrics and when they were loaded.
type Cache = Mutex<HashMap<String, (Arc<Derived>, Instant)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The org's derived metrics (name → formula), cached for [`TTL`].
pub(crate) async fn org_derived(st: &AppState, org: &str) -> Result<Arc<Derived>, ApiError> {
    if let Some((d, at)) = cache().lock().unwrap().get(org) {
        if at.elapsed() < TTL {
            return Ok(d.clone());
        }
    }
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, formula FROM derived_metrics WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let d = Arc::new(rows.into_iter().collect::<Derived>());
    cache().lock().unwrap().insert(org.to_string(), (d.clone(), Instant::now()));
    Ok(d)
}

/// Whether `metric` is one of the org's derived metrics.
pub(crate) async fn is_derived(st: &AppState, org: &str, metric: &str) -> Result<bool, ApiError> {
    Ok(org_derived(st, org).await?.contains_key(metric))
}

/// [`expand`] with the org's derived metrics; an invalid use is a bad request.
pub(crate) async fn resolve_text(st: &AppState, org: &str, text: &str) -> Result<String, ApiError> {
    let derived = org_derived(st, org).await?;
    expand(text, &derived).map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))
}

/// After a write: drops the org's derived metrics here and its metric catalog (which lists them)
/// on every replica.
async fn forget(st: &AppState, org: &str) {
    cache().lock().unwrap().remove(org);
    st.cache.invalidate(super::metrics::CATALOG, org).await;
}

// ================================================================ API

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DerivedMetric {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The metric queries, by letter (a, b, …).
    pub queries: Vec<String>,
    /// Arithmetic over the query letters: `a / b * 100`.
    pub expression: String,
    /// What queries of the name evaluate: the expression with the queries in place of the letters.
    pub formula: String,
    pub created_by: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DerivedMetricInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub queries: Vec<String>,
    pub expression: String,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    description: String,
    queries: serde_json::Value,
    expression: String,
    formula: String,
    created_by: String,
    created_at: i64,
    updated_at: i64,
}

impl From<Row> for DerivedMetric {
    fn from(r: Row) -> Self {
        DerivedMetric {
            id: r.id,
            name: r.name,
            description: r.description,
            queries: serde_json::from_value(r.queries).unwrap_or_default(),
            expression: r.expression,
            formula: r.formula,
            created_by: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// `checkout.error_ratio`: starts with a letter; letters, digits, `_` and `.`.
fn valid_name(n: &str) -> bool {
    n.starts_with(|c: char| c.is_ascii_alphabetic()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !n.ends_with('.') && !n.contains("..")
}

/// Trims and checks an input; returns its compiled formula. `others` are the org's other derived
/// metrics (a formula can't use one).
fn validate(input: &mut DerivedMetricInput, others: &Derived) -> Result<String, String> {
    input.name = input.name.trim().to_string();
    input.description = input.description.trim().to_string();
    input.expression = input.expression.trim().to_string();
    input.queries = input.queries.iter().map(|q| q.trim().to_string()).collect();
    if input.name.len() > MAX_NAME_CHARS || !valid_name(&input.name) {
        return Err(format!("Metric names are up to {MAX_NAME_CHARS} letters, digits, underscores and dots, starting with a letter (e.g. checkout.error_ratio)"));
    }
    if let Some(prefix) = RESERVED_PREFIXES.iter().find(|p| input.name.starts_with(*p)) {
        return Err(format!("Metric names can't start with '{prefix}'"));
    }
    if input.description.chars().count() > 1_000 {
        return Err("The description is at most 1,000 characters".into());
    }
    if input.queries.iter().any(|q| q.chars().count() > MAX_QUERY_CHARS) {
        return Err(format!("Queries are at most {MAX_QUERY_CHARS} characters"));
    }
    for q in &input.queries {
        let metric = parse_metric_query(q).map(|q| q.metric).unwrap_or_default();
        if metric == input.name || others.contains_key(&metric) {
            return Err(format!("'{metric}' is a derived metric: derived metrics can't be nested"));
        }
    }
    compile(&input.queries, &input.expression)
}

const COLS: &str = "id::text AS id, name, description, queries, expression, formula, created_by, \
                    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

async fn get_derived(st: &AppState, org: &str, id: &str) -> Result<DerivedMetric, ApiError> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM derived_metrics WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(DerivedMetric::from).ok_or_else(|| ApiError::NotFound("Derived metric not found".into()))
}

/// The org's derived metrics except `except` (by name), fresh from Postgres.
async fn others(st: &AppState, org: &str, except: &str) -> Result<Derived, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, formula FROM derived_metrics WHERE org_id = $1 AND name <> $2").bind(org).bind(except).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().collect())
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A derived metric named '{name}' already exists")),
        _ => e.into(),
    }
}

#[utoipa::path(get, path = "/api/v1/metrics/derived", operation_id = "list_derived_metrics", responses((status = 200, body = Vec<DerivedMetric>)), tag = "metrics")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<DerivedMetric>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM derived_metrics WHERE org_id = $1 ORDER BY name")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(DerivedMetric::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/metrics/derived", operation_id = "create_derived_metric", request_body = DerivedMetricInput, responses((status = 200, body = DerivedMetric)), tag = "metrics")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<DerivedMetricInput>) -> ApiResult<DerivedMetric> {
    let formula = validate(&mut input, &others(&st, &ctx.org_id, "").await?).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM derived_metrics WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_DERIVED {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_DERIVED} derived metrics")));
    }
    let (id,): (String,) = sqlx::query_as("INSERT INTO derived_metrics (org_id, name, description, queries, expression, formula, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(serde_json::json!(input.queries))
        .bind(&input.expression)
        .bind(&formula)
        .bind(&ctx.name)
        .fetch_one(&st.postgres)
        .await
        .map_err(|e| unique(e, &input.name))?;
    forget(&st, &ctx.org_id).await;
    ctx.audit(&st, "derived_metric.created", &input.name, &formula).await;
    Ok(Json(get_derived(&st, &ctx.org_id, &id).await?))
}

/// Changes the description, queries or formula; the name stays (dashboards and monitors use it).
#[utoipa::path(put, path = "/api/v1/metrics/derived/{id}", operation_id = "update_derived_metric", params(("id" = String, Path)), request_body = DerivedMetricInput, responses((status = 200, body = DerivedMetric)), tag = "metrics")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<DerivedMetricInput>) -> ApiResult<DerivedMetric> {
    let current = get_derived(&st, &ctx.org_id, &id).await?;
    let formula = validate(&mut input, &others(&st, &ctx.org_id, &current.name).await?).map_err(ApiError::BadRequest)?;
    if current.name != input.name {
        return Err(ApiError::BadRequest("A derived metric can't be renamed: create a new one".into()));
    }
    sqlx::query("UPDATE derived_metrics SET description = $3, queries = $4, expression = $5, formula = $6, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.description)
        .bind(serde_json::json!(input.queries))
        .bind(&input.expression)
        .bind(&formula)
        .execute(&st.postgres)
        .await?;
    forget(&st, &ctx.org_id).await;
    ctx.audit(&st, "derived_metric.updated", &input.name, &formula).await;
    Ok(Json(get_derived(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/metrics/derived/{id}", operation_id = "delete_derived_metric", params(("id" = String, Path)), responses((status = 204)), tag = "metrics")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM derived_metrics WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Derived metric not found".into())) };
    forget(&st, &ctx.org_id).await;
    ctx.audit(&st, "derived_metric.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derived(pairs: &[(&str, &str)]) -> Derived {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn letters_compile_to_a_formula_of_the_queries() {
        let q = strings(&["sum:checkout.errors{*} by {host}", "sum:checkout.requests{*} by {host}"]);
        assert_eq!(compile(&q, "a / b * 100").unwrap(), "(sum:checkout.errors{*} by {host}) / (sum:checkout.requests{*} by {host}) * 100");
        assert_eq!(compile(&q, "default_zero(a) + 1").unwrap(), "default_zero((sum:checkout.errors{*} by {host})) + 1");
        assert!(compile(&q, "a / c").is_err(), "c is not a query");
        assert!(compile(&q, "a; drop").is_err());
        assert!(compile(&q, "").is_err());
        assert!(compile(&[], "a").is_err());
        assert!(compile(&strings(&["median:x{*}"]), "a").is_err());
    }

    #[test]
    fn a_name_without_derived_metrics_is_left_alone() {
        let d = derived(&[("checkout.error_ratio", "sum:a{*} / sum:b{*}")]);
        assert_eq!(expand("avg:system.load.1{env:prod} by {host}", &d).unwrap(), "avg:system.load.1{env:prod} by {host}");
        assert_eq!(expand("avg:x{*}", &Derived::new()).unwrap(), "avg:x{*}");
    }

    #[test]
    fn a_derived_name_resolves_to_its_formula_with_the_querys_filters_and_group_by() {
        let d = derived(&[("checkout.error_ratio", "(sum:checkout.errors{status:5xx} by {host}) / (sum:checkout.requests{*} by {host}) * 100")]);
        assert_eq!(
            expand("avg:checkout.error_ratio{env:prod} by {service}", &d).unwrap(),
            "((sum:checkout.errors{status:5xx,env:prod} by {service}) / (sum:checkout.requests{env:prod} by {service}) * 100)"
        );
        // Without a by, the formula's own groups stay.
        assert_eq!(expand("sum:checkout.error_ratio{*}", &d).unwrap(), "((sum:checkout.errors{status:5xx} by {host}) / (sum:checkout.requests{*} by {host}) * 100)");
        // Inside a larger formula, next to plain queries.
        assert_eq!(
            expand("sum:checkout.error_ratio{env:qa} - avg:x{*}", &d).unwrap(),
            "((sum:checkout.errors{status:5xx,env:qa} by {host}) / (sum:checkout.requests{env:qa} by {host}) * 100) - avg:x{*}"
        );
        // The result is a formula the evaluator reads.
        let f = parse_formula(&expand("avg:checkout.error_ratio{env:prod} by {service}", &d).unwrap()).unwrap();
        assert_eq!(f.queries, vec!["sum:checkout.errors{status:5xx,env:prod} by {service}", "sum:checkout.requests{env:prod} by {service}"]);
    }

    #[test]
    fn derived_metrics_dont_nest_or_roll_up() {
        let d = derived(&[("a.ratio", "sum:x{*} / sum:y{*}"), ("b.ratio", "sum:a.ratio{*} * 2")]);
        assert!(expand("avg:b.ratio{*}", &d).unwrap_err().contains("nested"));
        assert!(expand("avg:a.ratio{*}.rollup(max)", &d).is_err() || parse_formula("avg:a.ratio{*}.rollup(max)").is_err());
    }

    #[test]
    fn inputs_are_checked() {
        let mut ok = DerivedMetricInput { name: " checkout.error_ratio ".into(), description: String::new(), queries: strings(&[" sum:a{*} ", "sum:b{*}"]), expression: " a / b ".into() };
        assert_eq!(validate(&mut ok, &Derived::new()).unwrap(), "(sum:a{*}) / (sum:b{*})");
        assert_eq!(ok.name, "checkout.error_ratio");
        for bad in ["", "1x", "x y", "trace.x", "x.."] {
            let mut i = DerivedMetricInput { name: bad.into(), description: String::new(), queries: strings(&["sum:a{*}"]), expression: "a".into() };
            assert!(validate(&mut i, &Derived::new()).is_err(), "{bad:?}");
        }
        let mut nested = DerivedMetricInput { name: "x.ratio".into(), description: String::new(), queries: strings(&["sum:other.ratio{*}"]), expression: "a".into() };
        assert!(validate(&mut nested, &derived(&[("other.ratio", "sum:a{*}")])).is_err());
        let mut itself = DerivedMetricInput { name: "x.ratio".into(), description: String::new(), queries: strings(&["sum:x.ratio{*}"]), expression: "a".into() };
        assert!(validate(&mut itself, &Derived::new()).is_err());
    }
}
