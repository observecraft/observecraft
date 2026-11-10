//! Log analytics: Datadog's `POST /api/v2/logs/analytics/aggregate` request and response shapes, which the
//! integrations' dashboards ask for (counts, unique counts, sums, averages, percentiles of a measure; by none
//! or several facets; as totals or over time). Served to the UI at `/api/v1/logs/aggregate`.

use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{parse_query, LogsSchema, Param, Schema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::routes::logs::{bind, scope, TimeRange};
use crate::state::{AppState, OrgId};

#[derive(Deserialize, ToSchema, Default)]
pub struct AggregateRequest {
    /// What to compute; a log count when empty.
    #[serde(default)]
    pub compute: Vec<Compute>,
    #[serde(default)]
    pub filter: AggregateFilter,
    /// Facets to split by, outermost first.
    #[serde(default)]
    pub group_by: Vec<GroupBy>,
}

#[derive(Deserialize, ToSchema, Clone)]
pub struct Compute {
    /// `count`, `cardinality`, `sum`, `min`, `max`, `avg`, `median`, `pc75`, `pc90`, `pc95`, `pc98`, `pc99`.
    pub aggregation: String,
    /// The facet or measure (`@duration`); none for `count`.
    pub metric: Option<String>,
    /// `total` (default) or `timeseries`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// A timeseries' step: `30s`, `5m`, `1h`, `1d` or milliseconds.
    #[schema(value_type = Option<String>)]
    pub interval: Option<Value>,
}

#[derive(Deserialize, ToSchema, Default)]
pub struct AggregateFilter {
    #[serde(default)]
    pub query: String,
    /// Epoch milliseconds; Datadog's ISO 8601 and `now` / `now-15m` strings too. The last 15 minutes by default.
    #[schema(value_type = Option<i64>)]
    pub from: Option<Value>,
    #[schema(value_type = Option<i64>)]
    pub to: Option<Value>,
}

#[derive(Deserialize, ToSchema, Clone)]
pub struct GroupBy {
    pub facet: String,
    /// Groups kept (default 10).
    pub limit: Option<i64>,
    pub sort: Option<GroupSort>,
}

#[derive(Deserialize, ToSchema, Clone)]
pub struct GroupSort {
    /// `desc` (default) or `asc`.
    pub order: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct AggregateResponse {
    pub data: AggregateData,
    #[schema(value_type = Object)]
    pub meta: Value,
}

#[derive(Serialize, ToSchema)]
pub struct AggregateData {
    pub buckets: Vec<Bucket>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct Bucket {
    /// Each facet's value (`{"@http.status_code": "500"}`).
    pub by: BTreeMap<String, String>,
    /// `c0`, `c1`, … as the computes: a number, or `[{time, value}]` for a timeseries.
    #[schema(value_type = Object)]
    pub computes: BTreeMap<String, Value>,
}

/// Rows read at most (groups × steps).
const MAX_ROWS: i64 = 100_000;
/// Groups kept at most, whatever the request's limits multiply to.
const MAX_GROUPS: usize = 1_000;
/// A timeseries without an interval gets about this many steps.
const DEFAULT_STEPS: i64 = 60;

/// Epoch milliseconds of Datadog's time forms, with `now_ms` for `now` and relative times.
pub fn parse_time(v: Option<&Value>, now_ms: i64, default_ms: i64) -> Result<i64, ApiError> {
    let bad = |s: &str| ApiError::BadRequest(format!("Unsupported time '{s}': use epoch milliseconds, ISO 8601 or now-15m"));
    match v {
        None => Ok(default_ms),
        Some(Value::Number(n)) => n.as_i64().ok_or_else(|| bad(&n.to_string())),
        Some(Value::String(s)) => {
            let s = s.trim();
            if let Some(rest) = s.strip_prefix("now") {
                return match rest.strip_prefix('-') {
                    None if rest.is_empty() => Ok(now_ms),
                    Some(d) => duration_ms(d).map(|d| now_ms - d).ok_or_else(|| bad(s)),
                    None => Err(bad(s)),
                };
            }
            if let Ok(ms) = s.parse::<i64>() {
                return Ok(ms);
            }
            chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp_millis()).map_err(|_| bad(s))
        }
        Some(other) => Err(bad(&other.to_string())),
    }
}

/// `30s`, `5m`, `1h`, `2d`, `1w` in milliseconds.
fn duration_ms(s: &str) -> Option<i64> {
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = n.parse().ok()?;
    let ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return None,
    };
    Some(n * ms)
}

/// The SQL aggregate of one compute, as a Float64 (NaN when it has no value).
fn compute_sql(c: &Compute) -> Result<(String, Option<Param>), ApiError> {
    let metric = || c.metric.as_deref().filter(|m| !m.is_empty() && *m != "*").ok_or_else(|| ApiError::BadRequest(format!("{} needs a metric (a facet or measure)", c.aggregation)));
    let number = |m: &str| {
        let (expr, p) = LogsSchema.field(m);
        (format!("toFloat64OrNull(toString({expr}))"), p)
    };
    let quantile = |q: &str| -> Result<(String, Option<Param>), ApiError> {
        let (x, p) = number(metric()?);
        Ok((format!("quantile({q})({x})"), p))
    };
    let (expr, p) = match c.aggregation.as_str() {
        "count" => ("count()".to_string(), None),
        "cardinality" => {
            let (expr, p) = LogsSchema.field(metric()?);
            (format!("uniqIf(toString({expr}), toString({expr}) != '')"), p)
        }
        "sum" | "min" | "max" | "avg" => {
            let (x, p) = number(metric()?);
            (format!("{}({x})", c.aggregation), p)
        }
        "median" => quantile("0.5")?,
        "pc75" => quantile("0.75")?,
        "pc90" => quantile("0.9")?,
        "pc95" => quantile("0.95")?,
        "pc98" => quantile("0.98")?,
        "pc99" => quantile("0.99")?,
        other => return Err(ApiError::BadRequest(format!("Unsupported aggregation '{other}'"))),
    };
    Ok((format!("ifNull(toFloat64({expr}), nan)"), p))
}

/// A timeseries' step in seconds: the first timeseries compute's interval, or about 60 steps over the range.
fn step_seconds(computes: &[Compute], range: TimeRange) -> Result<Option<i64>, ApiError> {
    let Some(c) = computes.iter().find(|c| c.kind.as_deref() == Some("timeseries")) else { return Ok(None) };
    let ms = match &c.interval {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => duration_ms(s).or_else(|| s.parse().ok()),
        _ => None,
    }
    .unwrap_or_else(|| (range.to - range.from) / DEFAULT_STEPS);
    Ok(Some((ms / 1000).max(1)))
}

/// The query: one row per group (and step) with the group's values and every compute.
pub fn aggregate_sql(req: &AggregateRequest, table: &str, clause: &str, scope_params: Vec<Param>, range: TimeRange) -> Result<(String, Vec<Param>), ApiError> {
    let computes = if req.compute.is_empty() { vec![Compute { aggregation: "count".into(), metric: None, kind: None, interval: None }] } else { req.compute.clone() };
    let mut params = Vec::new();
    let mut groups = Vec::new();
    for g in &req.group_by {
        let (expr, p) = LogsSchema.field(&g.facet);
        params.extend(p.clone());
        // A facet inside arrays of objects: the attribute itself, else each element's value (one row per value).
        match LogsSchema.element_values(&g.facet) {
            Some((elements, ep)) => {
                groups.push(format!("toString(arrayJoin(if({expr} != '', [{expr}], {elements})))"));
                params.extend(p);
                params.extend(ep);
            }
            None => groups.push(format!("toString({expr})")),
        }
    }
    let step = step_seconds(&computes, range)?;
    let t = match step {
        Some(s) => {
            params.push(Param::Int(s));
            "toInt64(toUnixTimestamp(toStartOfInterval(timestamp, toIntervalSecond(?)))) * 1000".to_string()
        }
        None => "toInt64(0)".to_string(),
    };
    let mut values = Vec::new();
    for c in &computes {
        let (expr, p) = compute_sql(c)?;
        values.push(expr);
        params.extend(p);
    }
    params.extend(scope_params);
    // A log without a group's facet is left out of the groups (Datadog's default).
    let mut present = String::new();
    for g in req.group_by.iter().filter(|g| LogsSchema.element_values(&g.facet).is_none()) {
        let (expr, p) = LogsSchema.field(&g.facet);
        present.push_str(&format!(" AND toString({expr}) != ''"));
        params.extend(p);
    }
    params.push(Param::Int(MAX_ROWS));
    let sql = format!(
        "SELECT [{}] AS g, {t} AS t, [{}] AS v FROM {table} WHERE {clause}{present} GROUP BY g, t ORDER BY t LIMIT ?",
        groups.join(", "),
        values.join(", ")
    );
    Ok((sql, params))
}

#[derive(clickhouse::Row, Deserialize, Debug, Clone)]
pub struct AggregateRow {
    pub g: Vec<String>,
    pub t: i64,
    pub v: Vec<f64>,
}

/// A number as JSON (NaN, a compute without a value, is null).
fn number(v: f64) -> Value {
    if v.is_finite() {
        crate::pipelines::grok::num(v).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

/// The response's buckets: the groups ranked by their first compute (over all steps), kept up to the limits.
pub fn buckets(req: &AggregateRequest, rows: Vec<AggregateRow>, timeseries: bool) -> Vec<Bucket> {
    let computes = req.compute.len().max(1);
    let mut by_group: HashMap<Vec<String>, Vec<AggregateRow>> = HashMap::new();
    let mut order: Vec<Vec<String>> = Vec::new();
    for r in rows {
        if !by_group.contains_key(&r.g) {
            order.push(r.g.clone());
        }
        by_group.entry(r.g.clone()).or_default().push(r);
    }
    let rank = |g: &Vec<String>| by_group[g].iter().map(|r| r.v.first().copied().filter(|x| x.is_finite()).unwrap_or(0.0)).sum::<f64>();
    let asc = req.group_by.first().and_then(|g| g.sort.as_ref()).and_then(|s| s.order.as_deref()) == Some("asc");
    order.sort_by(|a, b| {
        let (x, y) = (rank(a), rank(b));
        let o = if asc { x.partial_cmp(&y) } else { y.partial_cmp(&x) };
        o.unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.cmp(b))
    });
    let limit = req.group_by.iter().map(|g| g.limit.unwrap_or(10).clamp(1, 10_000) as usize).fold(1usize, |a, l| a.saturating_mul(l)).min(MAX_GROUPS);
    order.truncate(if req.group_by.is_empty() { 1 } else { limit });
    order
        .into_iter()
        .map(|g| {
            let rows = &by_group[&g];
            let by = req.group_by.iter().zip(&g).map(|(f, v)| (f.facet.clone(), v.clone())).collect();
            let computes = (0..computes)
                .map(|i| {
                    let value = if timeseries {
                        Value::Array(rows.iter().map(|r| json!({ "time": crate::pipelines::dates::to_rfc3339(r.t), "value": number(r.v.get(i).copied().unwrap_or(f64::NAN)) })).collect())
                    } else {
                        number(rows[0].v.get(i).copied().unwrap_or(f64::NAN))
                    };
                    (format!("c{i}"), value)
                })
                .collect();
            Bucket { by, computes }
        })
        .collect()
}

/// Log analytics, as Datadog's `/api/v2/logs/analytics/aggregate` (dashboard widgets over logs).
#[utoipa::path(post, path = "/api/v1/logs/aggregate", request_body = AggregateRequest,
    responses((status = 200, body = AggregateResponse)), tag = "logs")]
pub async fn aggregate(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess, Json(req): Json<AggregateRequest>) -> ApiResult<AggregateResponse> {
    let started = std::time::Instant::now();
    let now = chrono::Utc::now().timestamp_millis();
    let range = TimeRange { from: parse_time(req.filter.from.as_ref(), now, now - 900_000)?, to: parse_time(req.filter.to.as_ref(), now, now)? };
    range.validate()?;
    let terms = parse_query(&req.filter.query);
    let s = scope(&org, &access, range, &terms, None);
    let (sql, params) = aggregate_sql(&req, s.table, &s.clause, s.params, range)?;
    let rows = bind(st.ch(&org).query(&sql), &params).fetch_all::<AggregateRow>().await?;
    let timeseries = req.compute.iter().any(|c| c.kind.as_deref() == Some("timeseries"));
    Ok(Json(AggregateResponse {
        data: AggregateData { buckets: buckets(&req, rows, timeseries) },
        meta: json!({ "status": "done", "elapsed": started.elapsed().as_millis() as u64 }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(v: Value) -> AggregateRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn datadog_time_forms() {
        let now = 1_792_933_200_000;
        assert_eq!(parse_time(None, now, 7).unwrap(), 7);
        assert_eq!(parse_time(Some(&json!("now")), now, 0).unwrap(), now);
        assert_eq!(parse_time(Some(&json!("now-15m")), now, 0).unwrap(), now - 900_000);
        assert_eq!(parse_time(Some(&json!(123)), now, 0).unwrap(), 123);
        assert_eq!(parse_time(Some(&json!("2026-10-25T13:00:00Z")), now, 0).unwrap(), now);
        assert!(parse_time(Some(&json!("yesterday")), now, 0).is_err());
    }

    #[test]
    fn the_query_groups_by_every_facet_and_computes_each_aggregation() {
        let r = req(json!({
            "compute": [{"aggregation": "count"}, {"aggregation": "cardinality", "metric": "@usr.id"}, {"aggregation": "pc95", "metric": "@duration"}],
            "filter": {"query": "source:nginx"},
            "group_by": [{"facet": "service"}, {"facet": "@status_code"}]
        }));
        let (sql, params) = aggregate_sql(&r, "oc.logs", "org_id = ?", vec![Param::Str("o".into())], TimeRange { from: 0, to: 3_600_000 }).unwrap();
        assert!(sql.starts_with("SELECT [toString(service), toString(attr[?])] AS g, toInt64(0) AS t, [ifNull(toFloat64(count()), nan), "), "{sql}");
        assert!(sql.contains("uniqIf(toString(attr[?]), toString(attr[?]) != '')"), "{sql}");
        assert!(sql.contains("quantile(0.95)(toFloat64OrNull(toString(attr[?])))"), "{sql}");
        assert!(sql.contains("WHERE org_id = ? AND toString(service) != '' AND toString(attr[?]) != '' GROUP BY g, t"), "{sql}");
        // Parameters in the order of their placeholders: the facets, the metrics, the scope, the facets again, the limit.
        let strs: Vec<String> = params.iter().map(|p| match p { Param::Str(s) => s.clone(), Param::Int(i) => i.to_string() }).collect();
        assert_eq!(strs, ["status_code", "usr.id", "duration", "o", "status_code", &MAX_ROWS.to_string()]);
    }

    /// A facet inside an array of objects (`@install_targets.name`) groups each element's value, as Datadog does:
    /// the attribute itself when the log has it, else every element's (a log counts in each group).
    #[test]
    fn a_facet_inside_arrays_of_objects_groups_each_element() {
        let r = req(json!({"group_by": [{"facet": "@install_targets.name"}, {"facet": "@warning"}]}));
        let (sql, params) = aggregate_sql(&r, "oc.logs", "org_id = ?", vec![Param::Str("o".into())], TimeRange { from: 0, to: 3_600_000 }).unwrap();
        assert!(sql.starts_with("SELECT [toString(arrayJoin(if(attr[?] != '', [attr[?]], arrayFilter(_v -> _v != '', arrayConcat(arrayMap(_e -> "), "{sql}");
        assert!(sql.contains("WHERE org_id = ? AND toString(attr[?]) != '' GROUP BY g, t"), "the array facet's empty values are dropped by arrayJoin: {sql}");
        assert_eq!(sql.matches('?').count(), params.len());
    }

    /// On ClickHouse: Supply Chain Firewall's runs searched and grouped by `@install_targets.name` (an array of
    /// objects the normalizer stores as JSON), and a dotted key that is a plain attribute still matches.
    #[tokio::test]
    async fn arrays_of_objects_are_searched_and_grouped_on_clickhouse() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let ch = live.client.clone().with_database("oc");
        let org = format!("array-facets-{}-{}", std::process::id(), crate::routes::metrics::now_ms());
        let now = chrono::Utc::now().timestamp_millis();
        let targets = [r#"[{"name":"urllib3","version":"2.0.2"},{"name":"requests","version":"2.32.0"}]"#, r#"[{"name":"urllib3","version":"1.26.0"}]"#, "[INFO] not json", ""];
        for (i, t) in targets.iter().enumerate() {
            let attr = if t.is_empty() { "map('evt.outcome', 'ALLOW')".to_string() } else { format!("map('install_targets', '{}', 'evt.outcome', 'ALLOW')", t.replace('\'', "\\'")) };
            ch.query(&format!("INSERT INTO oc.logs (org_id, timestamp, status, service, host, source, message, tags, attr, retention_days) VALUES ('{org}', fromUnixTimestamp64Milli(toInt64({})), 'info', 'scfw', 'h', 'scfw', 'run', map(), {attr}, 15)", now - i as i64))
                .execute()
                .await
                .expect("insert");
        }
        let count = |q: &'static str| {
            let ch = ch.clone();
            let org = org.clone();
            async move {
                let f = oc_query::build_where(&LogsSchema, &parse_query(q), None);
                let mut params = vec![Param::Str(org)];
                params.extend(f.params);
                bind(ch.query(&format!("SELECT count() FROM oc.logs WHERE org_id = ? AND {}", f.clause)), &params).fetch_one::<u64>().await.unwrap_or_else(|e| panic!("{q}: {e}"))
            }
        };
        assert_eq!(count("@install_targets.name:urllib3").await, 2);
        assert_eq!(count("@install_targets.name:req*").await, 1);
        assert_eq!(count("-@install_targets.name:requests").await, 3);
        assert_eq!(count("@install_targets.version:2.0.2 @evt.outcome:ALLOW").await, 1);
        let r = req(json!({"group_by": [{"facet": "@install_targets.name"}]}));
        let (sql, params) = aggregate_sql(&r, "oc.logs", "org_id = ?", vec![Param::Str(org.clone())], TimeRange { from: 0, to: now + 1 }).unwrap();
        let rows = bind(ch.query(&sql), &params).fetch_all::<AggregateRow>().await.unwrap_or_else(|e| panic!("{sql}: {e}"));
        let mut groups: Vec<(String, f64)> = rows.into_iter().map(|r| (r.g[0].clone(), r.v[0])).collect();
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(groups, [("requests".to_string(), 1.0), ("urllib3".to_string(), 2.0)]);
    }

    #[test]
    fn a_timeseries_steps_by_its_interval() {
        let r = req(json!({"compute": [{"aggregation": "sum", "metric": "@bytes", "type": "timeseries", "interval": "5m"}]}));
        let (sql, params) = aggregate_sql(&r, "oc.logs", "1", vec![], TimeRange { from: 0, to: 3_600_000 }).unwrap();
        assert!(sql.contains("toStartOfInterval(timestamp, toIntervalSecond(?))"), "{sql}");
        assert!(matches!(params[0], Param::Int(300)));
        assert!(aggregate_sql(&req(json!({"compute": [{"aggregation": "avg"}]})), "t", "1", vec![], TimeRange { from: 0, to: 1 }).is_err(), "avg needs a metric");
        assert!(aggregate_sql(&req(json!({"compute": [{"aggregation": "mode", "metric": "@x"}]})), "t", "1", vec![], TimeRange { from: 0, to: 1 }).is_err());
    }

    #[test]
    fn buckets_keep_the_top_groups_with_their_computes() {
        let r = req(json!({"compute": [{"aggregation": "count"}, {"aggregation": "avg", "metric": "@d"}], "group_by": [{"facet": "service", "limit": 2}]}));
        let row = |g: &str, n: f64, avg: f64| AggregateRow { g: vec![g.into()], t: 0, v: vec![n, avg] };
        let b = buckets(&r, vec![row("a", 5.0, 1.5), row("b", 9.0, f64::NAN), row("c", 1.0, 2.0)], false);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].by["service"], "b");
        assert_eq!(b[0].computes["c0"], json!(9));
        assert_eq!(b[0].computes["c1"], Value::Null, "no value is null");
        assert_eq!(b[1].computes["c1"], json!(1.5));
        // Without groups: one bucket.
        let total = buckets(&req(json!({})), vec![AggregateRow { g: vec![], t: 0, v: vec![42.0] }], false);
        assert_eq!(total, vec![Bucket { by: BTreeMap::new(), computes: BTreeMap::from([("c0".to_string(), json!(42))]) }]);
    }

    #[test]
    fn timeseries_buckets_list_their_steps() {
        let r = req(json!({"compute": [{"aggregation": "count", "type": "timeseries"}]}));
        let b = buckets(&r, vec![AggregateRow { g: vec![], t: 0, v: vec![1.0] }, AggregateRow { g: vec![], t: 60_000, v: vec![3.0] }], true);
        assert_eq!(b[0].computes["c0"], json!([{"time": "1970-01-01T00:00:00.000Z", "value": 1}, {"time": "1970-01-01T00:01:00.000Z", "value": 3}]));
    }
}
