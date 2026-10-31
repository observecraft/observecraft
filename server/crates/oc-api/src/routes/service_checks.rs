//! Monitoring → Check Summary: the service checks reporting in the past day and, per check, its
//! status per host (and tag set) over time. Service checks are the `service_check.status` gauge
//! the intake writes (0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN, tagged `check:<name>`); their latest
//! messages are in Postgres `service_check_messages`.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const METRIC: &str = "service_check.status";
/// A group: host and its other tags.
type GroupKey = (String, Vec<String>);
/// Groups (host + tags) shown per check, the worst first.
const MAX_GROUPS: usize = 200;
/// Status strip cells.
const BUCKETS: i64 = 48;

/// Words written in capitals in check names.
const ACRONYMS: [&str; 16] = ["api", "aws", "cpu", "dns", "gcp", "http", "https", "io", "ntp", "os", "sql", "ssh", "ssl", "tcp", "tls", "url"];

/// A readable name for a check: `datadog.agent.up` → `Datadog Agent Up`, `ntp.in_sync` → `NTP In Sync`.
pub(crate) fn display_name(check: &str) -> String {
    check
        .split(['.', '_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            if ACRONYMS.contains(&w.to_ascii_lowercase().as_str()) {
                w.to_ascii_uppercase()
            } else {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn valid_check(c: &str) -> bool {
    !c.is_empty() && c.len() <= 200 && c.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCheckEntry {
    /// The check as sent (`datadog.agent.up`).
    pub check: String,
    /// Its readable name.
    pub name: String,
    /// Hosts and tag sets reporting it.
    pub groups: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct CheckRow {
    check: String,
    groups: u64,
}

/// The service checks reported in the past day.
#[utoipa::path(get, path = "/api/v1/service-checks", operation_id = "list_service_checks", responses((status = 200, body = Vec<ServiceCheckEntry>)), tag = "monitors")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<ServiceCheckEntry>> {
    let rows = st
        .ch(&ctx.org_id)
        .query(
            "SELECT Attributes['check'] AS check, uniqExact(SeriesId) AS groups FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND MetricName = ? AND LastSeen > now() - INTERVAL 1 DAY AND check != '' GROUP BY check ORDER BY check",
        )
        .bind(METRIC)
        .fetch_all::<CheckRow>()
        .await?;
    Ok(Json(rows.into_iter().map(|r| ServiceCheckEntry { name: display_name(&r.check), check: r.check, groups: r.groups }).collect()))
}

#[derive(Deserialize, ToSchema)]
pub struct ServiceCheckRequest {
    pub check: String,
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema, Default, Debug, PartialEq)]
pub struct ServiceCheckCounts {
    pub ok: u64,
    pub warning: u64,
    pub critical: u64,
    pub unknown: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCheckGroup {
    pub host: String,
    /// The run's other tags (`key:value`), without `check`.
    pub tags: Vec<String>,
    /// The latest status (0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN).
    pub status: u8,
    /// The worst status per cell of the strip (null: no run), oldest first.
    pub strip: Vec<Option<u8>>,
    /// The host's latest message for the check, if it sent one.
    pub message: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCheckMessage {
    pub host: String,
    pub status: u8,
    pub message: String,
    /// Epoch milliseconds.
    pub at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCheckDetail {
    pub check: String,
    pub name: String,
    /// Groups by their latest status.
    pub counts: ServiceCheckCounts,
    /// Start of each strip cell (epoch ms) and the cell's length.
    pub buckets: Vec<i64>,
    pub bucket_ms: i64,
    pub groups: Vec<ServiceCheckGroup>,
    /// The most recent non-empty message.
    pub latest_message: Option<ServiceCheckMessage>,
}

/// Groups by their latest status.
fn counts(statuses: impl Iterator<Item = u8>) -> ServiceCheckCounts {
    let mut c = ServiceCheckCounts::default();
    for s in statuses {
        match s {
            0 => c.ok += 1,
            1 => c.warning += 1,
            2 => c.critical += 1,
            _ => c.unknown += 1,
        }
    }
    c
}

/// How bad a status is, for ordering: CRITICAL, WARNING, UNKNOWN, OK.
fn severity(status: u8) -> u8 {
    match status {
        2 => 3,
        1 => 2,
        3 => 1,
        _ => 0,
    }
}

/// The worst status per cell from (cell start, status) points; cells start at `buckets`.
fn strip(points: &[(i64, u8)], buckets: &[i64]) -> Vec<Option<u8>> {
    let mut out = vec![None; buckets.len()];
    for &(t, s) in points {
        if let Some(i) = buckets.iter().position(|b| *b == t) {
            out[i] = Some(match out[i] {
                Some(prev) if severity(prev) >= severity(s) => prev,
                _ => s,
            });
        }
    }
    out
}

#[derive(clickhouse::Row, Deserialize)]
struct PointRow {
    host: String,
    tags: Vec<String>,
    t: i64,
    statuses: Vec<u8>,
}

#[derive(clickhouse::Row, Deserialize)]
struct LatestRow {
    host: String,
    tags: Vec<String>,
    status: f64,
}

/// The series of the check: host and its other tags, joined to the points in the window.
const SERIES: &str = "SELECT SeriesId, any(ResourceAttributes['host.name']) AS host, \
     any(arraySort(arrayMap((k, v) -> concat(k, ':', v), mapKeys(mapFilter((k, v) -> k != 'check', Attributes)), mapValues(mapFilter((k, v) -> k != 'check', Attributes))))) AS tags \
     FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND MetricName = ? AND Attributes['check'] = ? AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000))) GROUP BY SeriesId";

/// One check's status per host and tag set over the range.
#[utoipa::path(post, path = "/api/v1/service-checks/detail", operation_id = "service_check_detail", request_body = ServiceCheckRequest, responses((status = 200, body = ServiceCheckDetail)), tag = "monitors")]
pub async fn detail(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<ServiceCheckRequest>) -> ApiResult<ServiceCheckDetail> {
    req.range.validate()?;
    if !valid_check(&req.check) {
        return Err(ApiError::BadRequest("invalid check name".into()));
    }
    let org = &ctx.org_id;
    let span = (req.range.to - req.range.from).max(60_000);
    let bucket_ms = (((span + BUCKETS - 1) / BUCKETS).max(10_000) + 999) / 1000 * 1000;
    let first = req.range.from.div_euclid(bucket_ms) * bucket_ms;
    let buckets: Vec<i64> = (0..).map(|i| first + i * bucket_ms).take_while(|t| *t <= req.range.to).collect();
    let step_s = bucket_ms / 1000;
    let ch = st.ch(org);
    let points = ch
        .query(&format!(
            "SELECT s.host AS host, s.tags AS tags, toInt64(toUnixTimestamp(toStartOfInterval(x.TimeUnix, INTERVAL {step_s} SECOND, 'UTC'))) * 1000 AS t, groupUniqArray(toUInt8(least(greatest(x.Value, 0), 3))) AS statuses \
             FROM otel.metrics_samples_read AS x INNER JOIN ({SERIES}) AS s ON x.SeriesId = s.SeriesId \
             WHERE x.OrgId = {{oc_org:String}} AND x.MetricName = ? AND x.TimeUnix >= fromUnixTimestamp64Milli(?) AND x.TimeUnix <= fromUnixTimestamp64Milli(?) \
             GROUP BY host, tags, t ORDER BY host, tags, t"
        ))
        .bind(METRIC)
        .bind(&req.check)
        .bind(req.range.from)
        .bind(METRIC)
        .bind(req.range.from)
        .bind(req.range.to)
        .fetch_all::<PointRow>();
    let latest = ch
        .query(&format!(
            "SELECT s.host AS host, s.tags AS tags, argMax(x.Value, x.TimeUnix) AS status \
             FROM otel.metrics_samples_read AS x INNER JOIN ({SERIES}) AS s ON x.SeriesId = s.SeriesId \
             WHERE x.OrgId = {{oc_org:String}} AND x.MetricName = ? AND x.TimeUnix >= fromUnixTimestamp64Milli(?) AND x.TimeUnix <= fromUnixTimestamp64Milli(?) \
             GROUP BY host, tags"
        ))
        .bind(METRIC)
        .bind(&req.check)
        .bind(req.range.from)
        .bind(METRIC)
        .bind(req.range.from)
        .bind(req.range.to)
        .fetch_all::<LatestRow>();
    let messages = sqlx::query_as::<_, (String, i16, String, i64)>(
        "SELECT host, status, message, (extract(epoch FROM updated_at) * 1000)::bigint FROM service_check_messages WHERE org_id = $1 AND check_name = $2 ORDER BY updated_at DESC LIMIT 1000",
    )
    .bind(org)
    .bind(&req.check)
    .fetch_all(&st.postgres);
    let (points, latest, messages) =
        tokio::try_join!(async { points.await.map_err(ApiError::from) }, async { latest.await.map_err(ApiError::from) }, async { messages.await.map_err(ApiError::from) })?;

    let mut by_group: BTreeMap<GroupKey, Vec<(i64, u8)>> = BTreeMap::new();
    for p in points {
        let cell = by_group.entry((p.host, p.tags)).or_default();
        cell.extend(p.statuses.into_iter().map(|s| (p.t, s)));
    }
    let message_of = |host: &str| messages.iter().find(|m| m.0 == host && !m.2.is_empty()).map(|m| m.2.clone());
    let mut groups: Vec<ServiceCheckGroup> = latest
        .into_iter()
        .map(|l| {
            let status = l.status.clamp(0.0, 3.0) as u8;
            let cells = by_group.get(&(l.host.clone(), l.tags.clone())).map(|p| strip(p, &buckets)).unwrap_or_else(|| vec![None; buckets.len()]);
            ServiceCheckGroup { message: message_of(&l.host), host: l.host, tags: l.tags, status, strip: cells }
        })
        .collect();
    groups.sort_by(|a, b| severity(b.status).cmp(&severity(a.status)).then_with(|| a.host.cmp(&b.host)).then_with(|| a.tags.cmp(&b.tags)));
    let counts = counts(groups.iter().map(|g| g.status));
    groups.truncate(MAX_GROUPS);
    let latest_message = messages.iter().find(|m| !m.2.is_empty()).map(|m| ServiceCheckMessage { host: m.0.clone(), status: m.1.clamp(0, 3) as u8, message: m.2.clone(), at: m.3 });
    Ok(Json(ServiceCheckDetail { name: display_name(&req.check), check: req.check, counts, buckets, bucket_ms, groups, latest_message }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_names_read_as_titles() {
        assert_eq!(display_name("datadog.agent.up"), "Datadog Agent Up");
        assert_eq!(display_name("ntp.in_sync"), "NTP In Sync");
        assert_eq!(display_name("http.can_connect"), "HTTP Can Connect");
        assert_eq!(display_name("kubernetes.kubelet.check.ping"), "Kubernetes Kubelet Check Ping");
    }

    #[test]
    fn counts_and_strips_keep_the_worst_status() {
        assert_eq!(counts([0, 0, 1, 2, 3].into_iter()), ServiceCheckCounts { ok: 2, warning: 1, critical: 1, unknown: 1 });
        let cells = strip(&[(0, 0), (60, 1), (60, 2), (60, 0), (120, 3), (120, 1), (999, 2)], &[0, 60, 120, 180]);
        assert_eq!(cells, vec![Some(0), Some(2), Some(1), None], "critical over warning, warning over unknown; points outside cells are ignored");
    }

    #[test]
    fn check_names_are_validated() {
        assert!(valid_check("datadog.agent.up"));
        assert!(!valid_check(""));
        assert!(!valid_check("x' OR 1=1"));
    }
}
