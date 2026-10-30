//! Metrics → Catalog and Overview: per-metric volume (series in the range and in the period before
//! it, first submission), the volume over time, and one metric's details (tags and their values,
//! hosts, volume over time, and the dashboards, notebooks, monitors and SLOs that read it).
//!
//! Tenancy: ClickHouse queries filter on `OrgId = {oc_org:String}` through `AppState::ch(org)`;
//! Postgres queries on `org_id`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};
use oc_query::short_tag;

/// Cache namespace of each metric's first submission (a scan of the hourly rollups).
const FIRST_SEEN: &str = "metric_first_seen";
const FIRST_SEEN_TTL: Duration = Duration::from_secs(600);
/// Values listed per tag key in a metric's details.
const VALUES_PER_KEY: u64 = 50;
/// Related assets listed per kind.
const RELATED_LIMIT: i64 = 100;

#[derive(Deserialize, ToSchema)]
pub struct CatalogStatsRequest {
    pub range: TimeRange,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MetricStats {
    pub name: String,
    pub kind: String,
    /// Distinct series in the range.
    pub series: u64,
    /// Distinct series in the period of the same length before the range.
    pub previous_series: u64,
    /// Epoch ms of the first hour it was reported in (within retention); absent before its first rollup.
    pub first_seen: Option<i64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct VolumePoint {
    /// Bucket start, epoch ms.
    pub t: i64,
    /// Distinct series in the bucket.
    pub series: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStats {
    pub metrics: Vec<MetricStats>,
    /// Distinct series of all metrics per bucket (5 minutes up to two days, else hours).
    pub timeline: Vec<VolumePoint>,
}

#[derive(clickhouse::Row, Deserialize)]
struct CountRow {
    name: String,
    kind: String,
    series: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct NameCount {
    name: String,
    series: u64,
}

#[derive(clickhouse::Row, Deserialize, Serialize)]
struct FirstSeenRow {
    name: String,
    first: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct PointRow {
    t: i64,
    series: u64,
}

/// The rollup table whose buckets suit the range: 5-minute buckets up to two days, else hours.
fn rollup_table(range: &TimeRange) -> &'static str {
    if range.to - range.from <= 2 * 86_400_000 {
        "otel.metrics_rollup_5m"
    } else {
        "otel.metrics_rollup_1h"
    }
}

/// Distinct series per bucket in the range, of one metric or of all.
fn timeline_sql(range: &TimeRange, one_metric: bool) -> String {
    format!(
        "SELECT toInt64(toUnixTimestamp(Bucket)) * 1000 AS t, uniqExact(SeriesId) AS series FROM {} \
         WHERE OrgId = {{oc_org:String}}{} AND Bucket >= toDateTime(intDiv(?, 1000)) AND Bucket <= toDateTime(intDiv(?, 1000)) \
         GROUP BY Bucket ORDER BY Bucket",
        rollup_table(range),
        if one_metric { " AND MetricName = ?" } else { "" }
    )
}

/// Joins the range's counts, the previous period's and the first submissions, the biggest first.
fn merge_stats(current: Vec<CountRow>, previous: Vec<NameCount>, first: &[FirstSeenRow]) -> Vec<MetricStats> {
    let previous: HashMap<String, u64> = previous.into_iter().map(|r| (r.name, r.series)).collect();
    let first: HashMap<&str, i64> = first.iter().map(|r| (r.name.as_str(), r.first)).collect();
    let mut out: Vec<MetricStats> = current
        .into_iter()
        .map(|r| MetricStats {
            previous_series: previous.get(&r.name).copied().unwrap_or(0),
            first_seen: first.get(r.name.as_str()).copied(),
            name: r.name,
            kind: r.kind,
            series: r.series,
        })
        .collect();
    out.sort_by(|a, b| b.series.cmp(&a.series).then_with(|| a.name.cmp(&b.name)));
    out
}

async fn first_seen(st: &AppState, org: &str) -> Result<Arc<Vec<FirstSeenRow>>, ApiError> {
    st.cache
        .get_or_load(FIRST_SEEN, org, "", FIRST_SEEN_TTL, || async {
            st.ch(org)
                .query("SELECT MetricName AS name, toInt64(toUnixTimestamp(min(Bucket))) * 1000 AS first FROM otel.metrics_rollup_1h WHERE OrgId = {oc_org:String} GROUP BY MetricName")
                .fetch_all::<FirstSeenRow>()
                .await
                .map_err(ApiError::from)
        })
        .await
}

/// Every metric's series in the range and before it, its first submission, and the volume over time.
#[utoipa::path(post, path = "/api/v1/metrics/catalog/stats", operation_id = "metric_catalog_stats", request_body = CatalogStatsRequest, responses((status = 200, body = CatalogStats)), tag = "metrics")]
pub async fn stats(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CatalogStatsRequest>) -> ApiResult<CatalogStats> {
    req.range.validate()?;
    let span = req.range.to - req.range.from;
    let ch = st.ch(&org);
    let current = ch
        .query(
            "SELECT MetricName AS name, toString(any(MetricType)) AS kind, uniqExact(SeriesId) AS series FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000))) GROUP BY MetricName",
        )
        .bind(req.range.from)
        .fetch_all::<CountRow>();
    let previous = ch
        .query(&format!(
            "SELECT MetricName AS name, uniqExact(SeriesId) AS series FROM {} WHERE OrgId = {{oc_org:String}} \
             AND Bucket >= toDateTime(intDiv(?, 1000)) AND Bucket < toDateTime(intDiv(?, 1000)) GROUP BY MetricName",
            rollup_table(&req.range)
        ))
        .bind(req.range.from - span)
        .bind(req.range.from)
        .fetch_all::<NameCount>();
    let timeline = ch.query(&timeline_sql(&req.range, false)).bind(req.range.from).bind(req.range.to).fetch_all::<PointRow>();
    let (current, previous, timeline, first) = tokio::try_join!(
        async { current.await.map_err(ApiError::from) },
        async { previous.await.map_err(ApiError::from) },
        async { timeline.await.map_err(ApiError::from) },
        first_seen(&st, &org),
    )?;
    Ok(Json(CatalogStats {
        metrics: merge_stats(current, previous, &first),
        timeline: timeline.into_iter().map(|p| VolumePoint { t: p.t, series: p.series }).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct MetricDetailRequest {
    pub name: String,
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct MetricTagKey {
    pub key: String,
    /// Distinct values in the range.
    pub count: u64,
    /// Up to 50 of them, sorted.
    pub values: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct RelatedAsset {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, ToSchema)]
pub struct RelatedAssets {
    pub dashboards: Vec<RelatedAsset>,
    pub notebooks: Vec<RelatedAsset>,
    pub monitors: Vec<RelatedAsset>,
    pub slos: Vec<RelatedAsset>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricDetail {
    pub name: String,
    pub series: u64,
    /// Distinct hosts reporting it in the range.
    pub hosts: u64,
    /// Distinct tag values (all keys) in the range.
    pub tag_values: u64,
    pub first_seen: Option<i64>,
    pub tags: Vec<MetricTagKey>,
    pub timeline: Vec<VolumePoint>,
    pub related: RelatedAssets,
}

#[derive(clickhouse::Row, Deserialize)]
struct TagRow {
    key: String,
    count: u64,
    values: Vec<String>,
}

/// Metric names as the intake stores them: letters, digits, `_ . - :` and `/`.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':' | '/'))
}

/// A Postgres regex matching the metric name as a whole word (not `a.b` inside `a.b.c`).
fn name_regex(name: &str) -> String {
    let escaped: String = name.chars().flat_map(|c| if c.is_ascii_alphanumeric() || c == '_' { vec![c] } else { vec!['\\', c] }).collect();
    format!("(^|[^A-Za-z0-9_.]){escaped}([^A-Za-z0-9_.]|$)")
}

/// Tag keys under the names queries use (`k8s.namespace.name` → `kube_namespace`), counts and values merged.
fn merge_tag_keys(rows: Vec<TagRow>) -> Vec<MetricTagKey> {
    let mut keys: BTreeMap<String, (u64, Vec<String>)> = BTreeMap::new();
    for r in rows.into_iter().filter(|r| r.key != "oc.org_id") {
        let key = short_tag(&r.key).map(str::to_string).unwrap_or(r.key);
        let e = keys.entry(key).or_default();
        e.0 = e.0.max(r.count);
        e.1.extend(r.values);
    }
    keys.into_iter()
        .map(|(key, (count, mut values))| {
            values.sort();
            values.dedup();
            values.truncate(VALUES_PER_KEY as usize);
            MetricTagKey { key, count, values }
        })
        .collect()
}

/// One metric: its series, hosts and tag values in the range, its volume over time and where it is used.
#[utoipa::path(post, path = "/api/v1/metrics/detail", operation_id = "metric_detail", request_body = MetricDetailRequest, responses((status = 200, body = MetricDetail)), tag = "metrics")]
pub async fn detail(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<MetricDetailRequest>) -> ApiResult<MetricDetail> {
    req.range.validate()?;
    if !valid_name(&req.name) {
        return Err(ApiError::BadRequest("invalid metric name".into()));
    }
    let ch = st.ch(&org);
    let series = ch
        .query("SELECT uniqExact(SeriesId) FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND MetricName = ? AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000)))")
        .bind(&req.name)
        .bind(req.range.from)
        .fetch_one::<u64>();
    let tags = ch
        .query(&format!(
            "SELECT toString(Key) AS key, uniqExact(Value) AS count, arraySort(groupUniqArray({VALUES_PER_KEY})(Value)) AS values FROM otel.metrics_tags \
             WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000))) AND Value != '' GROUP BY Key ORDER BY key"
        ))
        .bind(&req.name)
        .bind(req.range.from)
        .fetch_all::<TagRow>();
    let timeline = ch
        .query(&timeline_sql(&req.range, true))
        .bind(&req.name)
        .bind(req.range.from)
        .bind(req.range.to)
        .fetch_all::<PointRow>();
    let (series, tags, timeline, first) = tokio::try_join!(
        async { series.await.map_err(ApiError::from) },
        async { tags.await.map_err(ApiError::from) },
        async { timeline.await.map_err(ApiError::from) },
        first_seen(&st, &org),
    )?;
    let tags = merge_tag_keys(tags);
    let hosts = tags.iter().find(|t| t.key == "host" || t.key == "host.name").map_or(0, |t| t.count);
    let tag_values = tags.iter().map(|t| t.count).sum();
    Ok(Json(MetricDetail {
        first_seen: first.iter().find(|r| r.name == req.name).map(|r| r.first),
        name: req.name.clone(),
        series,
        hosts,
        tag_values,
        tags,
        timeline: timeline.into_iter().map(|p| VolumePoint { t: p.t, series: p.series }).collect(),
        related: related(&st, &org, &req.name).await?,
    }))
}

/// Dashboards, notebooks, monitors and SLOs whose definition names the metric.
async fn related(st: &AppState, org: &str, name: &str) -> Result<RelatedAssets, ApiError> {
    let re = name_regex(name);
    let pg = &st.postgres;
    let fetch = |sql: &'static str| {
        let re = re.clone();
        async move {
            sqlx::query_as::<_, (String, String)>(sql)
                .bind(org)
                .bind(re)
                .bind(RELATED_LIMIT)
                .fetch_all(pg)
                .await
                .map_err(ApiError::from)
        }
    };
    let (dashboards, notebooks, monitors, slos) = tokio::try_join!(
        fetch("SELECT id, title FROM dashboards WHERE org_id = $1 AND deleted_at IS NULL AND definition::text ~ $2 ORDER BY title LIMIT $3"),
        fetch("SELECT id, title FROM notebooks WHERE org_id = $1 AND deleted_at IS NULL AND cells::text ~ $2 ORDER BY title LIMIT $3"),
        fetch("SELECT id::text, name FROM monitors WHERE org_id = $1 AND (query ~ $2 OR definition::text ~ $2) ORDER BY name LIMIT $3"),
        fetch("SELECT id::text, name FROM slos WHERE org_id = $1 AND (coalesce(good_query, '') ~ $2 OR coalesce(total_query, '') ~ $2 OR coalesce(slice_query, '') ~ $2) ORDER BY name LIMIT $3"),
    )?;
    let assets = |rows: Vec<(String, String)>| rows.into_iter().map(|(id, title)| RelatedAsset { id, title }).collect();
    Ok(RelatedAssets {
        dashboards: assets(dashboards),
        notebooks: assets(notebooks),
        monitors: assets(monitors),
        slos: assets(slos),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_join_the_previous_period_and_first_submission_biggest_first() {
        let current = vec![
            CountRow {
                name: "system.cpu.user".into(),
                kind: "gauge".into(),
                series: 4,
            },
            CountRow {
                name: "kubernetes.cpu.usage.total".into(),
                kind: "gauge".into(),
                series: 40,
            },
        ];
        let previous = vec![NameCount {
            name: "system.cpu.user".into(),
            series: 6,
        }];
        let first = vec![FirstSeenRow {
            name: "kubernetes.cpu.usage.total".into(),
            first: 1_000,
        }];
        let out = merge_stats(current, previous, &first);
        assert_eq!(
            out[0],
            MetricStats {
                name: "kubernetes.cpu.usage.total".into(),
                kind: "gauge".into(),
                series: 40,
                previous_series: 0,
                first_seen: Some(1_000)
            }
        );
        assert_eq!(out[1].previous_series, 6);
        assert_eq!(out[1].first_seen, None);
    }

    #[test]
    fn short_ranges_read_five_minute_buckets() {
        assert!(timeline_sql(&TimeRange { from: 0, to: 3_600_000 }, false).contains("metrics_rollup_5m"));
        assert!(timeline_sql(&TimeRange { from: 0, to: 7 * 86_400_000 }, true).contains("metrics_rollup_1h WHERE OrgId = {oc_org:String} AND MetricName = ?"));
    }

    #[test]
    fn metric_names_match_whole_words_only() {
        assert_eq!(name_regex("kubernetes.cpu.usage"), r"(^|[^A-Za-z0-9_.])kubernetes\.cpu\.usage([^A-Za-z0-9_.]|$)");
        assert_eq!(name_regex("a-b"), r"(^|[^A-Za-z0-9_.])a\-b([^A-Za-z0-9_.]|$)");
        assert!(!valid_name("a'; drop"));
        assert!(valid_name("kubernetes.cpu.usage.total"));
    }

    #[test]
    fn tag_keys_read_under_their_query_names_without_the_tenant_stamp() {
        let rows = vec![
            TagRow {
                key: "k8s.namespace.name".into(),
                count: 2,
                values: vec!["b".into(), "a".into()],
            },
            TagRow {
                key: "oc.org_id".into(),
                count: 1,
                values: vec!["demo".into()],
            },
            TagRow {
                key: "service".into(),
                count: 1,
                values: vec!["api".into()],
            },
        ];
        let keys = merge_tag_keys(rows);
        assert_eq!(keys.iter().map(|k| k.key.as_str()).collect::<Vec<_>>(), vec!["kube_namespace", "service"]);
        assert_eq!(keys[0].values, vec!["a", "b"]);
    }
}
