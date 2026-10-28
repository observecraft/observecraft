//! Per-series values of integration metrics, for the product pages built on them (GPU Monitoring,
//! Database Monitoring, Data Jobs): each series' labels and one value per window (or per interval),
//! normalized so the pages don't care who sent the metric:
//! - gauges (OpenTelemetry gauges, Datadog gauges and rates): the average;
//! - non-monotonic sums (OpenTelemetry up-down counters): the average;
//! - monotonic cumulative sums (OpenTelemetry counters): the per-second rate;
//! - monotonic delta sums (Datadog counts): the per-second rate over the window.
//!
//! Like the host list, these fleets are small (devices, database instances, jobs), so the pages
//! group and filter the series in memory.
//!
//! Tenancy: every query filters on `OrgId = {oc_org:String}` through `AppState::ch(org)`.

use std::collections::HashMap;

use serde::Deserialize;

use super::metrics::{name_list, points_subquery, with_window};
use crate::error::ApiError;
use crate::state::AppState;

/// Most series (× intervals) one page reads; beyond that the fleet needs server-side grouping.
const MAX_ROWS: u64 = 200_000;

/// One series' value in a window (or one interval of it).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeriesValue {
    pub metric: String,
    pub res: HashMap<String, String>,
    pub attrs: HashMap<String, String>,
    /// Start of the interval (epoch ms); 0 for a whole-window value.
    pub t: i64,
    pub value: f64,
    /// Largest point (a Datadog count's latest total, e.g. the Spark check's job counts).
    pub max: f64,
    /// Points behind the value.
    pub count: u64,
    /// Latest point (epoch ms).
    pub last: i64,
}

impl SeriesValue {
    /// The first non-empty value of `keys`, looked up in the data point attributes, then the resource.
    pub fn tag(&self, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| self.attrs.get(*k).or_else(|| self.res.get(*k)).filter(|v| !v.is_empty()).cloned())
    }
}

/// The value of a series per window: see the module docs. `window_ms` is the interval (or window) length.
fn value_expr(kind: &str, window_ms: i64) -> String {
    match kind {
        "gauge" => "avg(Value)".into(),
        _ => format!(
            "multiIf(NOT any(Monotonic), avg(Value), \
                     any(Temporality) = 2, if(count() > 1, greatest(max(Value) - min(Value), 0) * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1), nan), \
                     sum(Value) * 1000 / {window_ms})"
        ),
    }
}

/// The query for the `kind` series of `names` (constants), one row per series and interval of
/// `step_ms` (whole window when none). Parameters: the row limit.
pub(crate) fn series_values_sql(kind: &str, names: &[&str], window_ms: i64, step_ms: Option<i64>) -> String {
    let src = points_subquery(kind, names);
    let (t, per) = match step_ms {
        Some(step) => {
            let step_s = (step / 1000).max(1);
            (format!("toInt64(toUnixTimestamp(toStartOfInterval(TimeUnix, INTERVAL {step_s} SECOND))) * 1000"), step_s * 1000)
        }
        None => ("toInt64(0)".to_string(), window_ms.max(1)),
    };
    format!(
        "SELECT MetricName AS metric, toJSONString(ResourceAttributes) AS res, toJSONString(Attributes) AS attrs, {t} AS t, \
         toFloat64({value}) AS value, toFloat64(max(Value)) AS max, count() AS count, toInt64(toUnixTimestamp64Milli(max(TimeUnix))) AS last \
         FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName IN ({names}) \
         GROUP BY metric, res, attrs, t ORDER BY t LIMIT ?",
        value = value_expr(kind, per),
        names = name_list(names),
    )
}

#[derive(clickhouse::Row, Deserialize)]
struct Row {
    metric: String,
    res: String,
    attrs: String,
    t: i64,
    value: f64,
    max: f64,
    count: u64,
    last: i64,
}

fn to_value(r: Row) -> Option<SeriesValue> {
    r.value.is_finite().then(|| SeriesValue {
        metric: r.metric,
        res: serde_json::from_str(&r.res).unwrap_or_default(),
        attrs: serde_json::from_str(&r.attrs).unwrap_or_default(),
        t: r.t,
        value: r.value,
        max: r.max,
        count: r.count,
        last: r.last,
    })
}

/// Values of every gauge and sum series named `names` between `from` and `to` (epoch ms), per
/// interval of `step_ms` or for the whole window. Series without a value (a counter seen once) are left out.
pub(crate) async fn series_values(st: &AppState, org: &str, names: &[&str], from: i64, to: i64, step_ms: Option<i64>) -> Result<Vec<SeriesValue>, ApiError> {
    let window = to - from;
    let query = |kind: &str| with_window(st.ch(org), from, to).query(&series_values_sql(kind, names, window, step_ms)).bind(MAX_ROWS).fetch_all::<Row>();
    let (gauges, sums) = tokio::try_join!(query("gauge"), query("sum"))?;
    Ok(gauges.into_iter().chain(sums).filter_map(to_value).collect())
}

/// Interval for a chart of about `points` points over `range_ms`: at least the 10 s the Agent reports at.
pub(crate) fn chart_step(range_ms: i64, points: i64) -> i64 {
    const NICE: &[i64] = &[10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 900_000, 1_800_000, 3_600_000, 7_200_000, 14_400_000, 21_600_000, 43_200_000, 86_400_000];
    let raw = range_ms / points.max(1);
    NICE.iter().copied().find(|s| *s >= raw).unwrap_or(86_400_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_filters_by_org_and_takes_no_user_input() {
        let sql = series_values_sql("sum", &["postgresql.commits", "DCGM_FI_DEV_GPU_UTIL"], 600_000, None);
        assert!(sql.contains("OrgId = {oc_org:String}"));
        assert!(sql.contains("MetricName IN ('postgresql.commits', 'DCGM_FI_DEV_GPU_UTIL')"));
        // Counters become rates; the only bound value is the row limit.
        assert!(sql.contains("any(Temporality) = 2"));
        assert!(sql.contains("sum(Value) * 1000 / 600000"));
        assert_eq!(sql.matches('?').count(), 1);
        assert!(sql.contains("toInt64(0) AS t"));
    }

    #[test]
    fn stepped_query_groups_by_interval() {
        let sql = series_values_sql("gauge", &["gpu.temperature"], 3_600_000, Some(60_000));
        assert!(sql.contains("INTERVAL 60 SECOND"));
        assert!(sql.contains("avg(Value)"));
    }

    #[test]
    fn rows_parse_labels_and_drop_missing_values() {
        let row = |value: f64| Row {
            metric: "gpu.temperature".into(),
            res: r#"{"host.name":"i-0a1b2c3d4e5f60001"}"#.into(),
            attrs: r#"{"gpu_uuid":"GPU-1","gpu_index":""}"#.into(),
            t: 0,
            value,
            max: value,
            count: 3,
            last: 5,
        };
        let v = to_value(row(61.0)).unwrap();
        assert_eq!(v.tag(&["host.name"]).as_deref(), Some("i-0a1b2c3d4e5f60001"));
        assert_eq!(v.tag(&["gpu_index", "gpu_uuid"]).as_deref(), Some("GPU-1"), "empty values are skipped");
        assert!(to_value(row(f64::NAN)).is_none());
    }

    #[test]
    fn chart_steps_are_round() {
        assert_eq!(chart_step(3_600_000, 60), 60_000);
        assert_eq!(chart_step(900_000, 60), 30_000);
        assert_eq!(chart_step(60_000, 60), 10_000);
        assert_eq!(chart_step(7 * 86_400_000, 60), 14_400_000);
    }
}
