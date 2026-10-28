//! Watchdog: anomalies found automatically in the telemetry we already store, without monitors.
//!
//! Computed on request over the past day in 5-minute buckets: APM error rate and p95 latency per
//! service (entry spans), error logs per service (through the reader's log access) and CPU, memory
//! and disk usage per host. Each bucket is compared with a robust baseline (median + k·MAD of the
//! previous normal buckets); consecutive anomalous buckets form an alert, which is ongoing while
//! the series stays out of range, resolved once it returns, and expired when the series stops.

use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::apm::ENTRY;
use super::logs::{bind, scope, TimeRange};
use super::metrics::{points_subquery, with_window};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::{AppState, OrgId};

/// Bucket length.
pub(crate) const BUCKET_MS: i64 = 5 * 60_000;
/// Buckets analysed (the past day).
pub(crate) const WINDOW_BUCKETS: usize = 288;
/// Buckets the baseline is taken from before each analysed bucket (2 h).
pub(crate) const BASELINE_BUCKETS: usize = 24;
/// A bucket is only judged with at least this many baseline buckets.
const MIN_BASELINE: usize = 12;
/// Deviations (robust standard deviations) above the median that count as anomalous.
const K: f64 = 4.0;
/// Anomalous buckets in a row an alert needs (10 minutes).
const MIN_ANOMALOUS: usize = 2;
/// Requests a bucket needs before its error rate or latency is judged.
const MIN_HITS: u64 = 10;
/// Services analysed (the busiest), and hosts.
const MAX_SERVICES: u64 = 50;
const MAX_HOSTS: usize = 200;
/// Buckets shown before and after an alert's anomalous window.
const CONTEXT_BUCKETS: usize = 24;
const MAX_ALERTS: usize = 200;

/// Host metrics read, in both naming schemes (Datadog Agent and OpenTelemetry hostmetrics).
const HOST_METRICS: &[&str] = &["system.cpu.idle", "system.cpu.utilization", "system.mem.pct_usable", "system.memory.utilization", "system.disk.in_use", "system.filesystem.utilization"];

/// A bucket's start (epoch ms) from a time column.
const BUCKET_T: &str = "toInt64(toUnixTimestamp(toStartOfInterval({col}, INTERVAL 300 SECOND, 'UTC'))) * 1000";

#[derive(Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchdogCategory {
    Apm,
    Infrastructure,
    Logs,
}

#[derive(Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchdogAlertType {
    ErrorRate,
    Latency,
    ErrorLogs,
    Cpu,
    Memory,
    Disk,
}

#[derive(Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchdogAlertStatus {
    /// The series is still out of its expected range.
    Ongoing,
    /// The series came back into its expected range.
    Resolved,
    /// The series stopped reporting before it came back.
    Expired,
}

/// How far above its baseline a series of a type must go: at least `min_delta` above the median
/// and `min_ratio` times it, besides K robust deviations.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rule {
    pub min_delta: f64,
    pub min_ratio: f64,
}

/// The expected range of a bucket from its baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Band {
    pub median: f64,
    pub lower: f64,
    pub upper: f64,
}

/// Anomalous buckets `start..=end` (indices into the analysed values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Episode {
    pub start: usize,
    pub end: usize,
    /// Highest value in the episode.
    pub peak: f64,
    /// The baseline median when it started.
    pub baseline: f64,
}

pub(crate) struct Detection {
    /// The expected range per bucket (None before enough baseline, or without data).
    pub bands: Vec<Option<Band>>,
    pub episodes: Vec<Episode>,
}

impl WatchdogAlertType {
    pub(crate) fn category(self) -> WatchdogCategory {
        match self {
            Self::ErrorRate | Self::Latency => WatchdogCategory::Apm,
            Self::ErrorLogs => WatchdogCategory::Logs,
            Self::Cpu | Self::Memory | Self::Disk => WatchdogCategory::Infrastructure,
        }
    }

    pub(crate) fn rule(self) -> Rule {
        match self {
            // Percent of requests.
            Self::ErrorRate => Rule { min_delta: 5.0, min_ratio: 2.0 },
            // p95 in ms.
            Self::Latency => Rule { min_delta: 50.0, min_ratio: 1.5 },
            // Error logs per bucket.
            Self::ErrorLogs => Rule { min_delta: 20.0, min_ratio: 3.0 },
            // Percent used.
            Self::Cpu => Rule { min_delta: 20.0, min_ratio: 1.0 },
            Self::Memory => Rule { min_delta: 10.0, min_ratio: 1.0 },
            Self::Disk => Rule { min_delta: 5.0, min_ratio: 1.0 },
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::ErrorRate => "error_rate",
            Self::Latency => "latency",
            Self::ErrorLogs => "error_logs",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
        }
    }

    fn unit(self) -> &'static str {
        match self {
            Self::Latency => "ms",
            Self::ErrorLogs => "logs",
            _ => "percent",
        }
    }

    /// What rose, as a title and as a monitor name.
    fn what(self) -> (&'static str, &'static str) {
        match self {
            Self::ErrorRate => ("Error rate increase", "Error rate"),
            Self::Latency => ("Latency increase", "p95 latency"),
            Self::ErrorLogs => ("Error logs increase", "Error log volume"),
            Self::Cpu => ("CPU usage increase", "CPU usage"),
            Self::Memory => ("Memory usage increase", "Memory usage"),
            Self::Disk => ("Disk usage increase", "Disk usage"),
        }
    }
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// The expected range from baseline values: median ± K·1.4826·MAD, the upper bound at least the rule's floor.
pub(crate) fn band(baseline: &[f64], rule: Rule) -> Option<Band> {
    if baseline.len() < MIN_BASELINE {
        return None;
    }
    let mut sorted = baseline.to_vec();
    sorted.sort_by(f64::total_cmp);
    let med = median(&sorted);
    let mut dev: Vec<f64> = sorted.iter().map(|v| (v - med).abs()).collect();
    dev.sort_by(f64::total_cmp);
    let spread = K * 1.4826 * median(&dev);
    let margin = spread.max(rule.min_delta).max(med * (rule.min_ratio - 1.0));
    Some(Band { median: med, lower: (med - spread).max(0.0), upper: med + margin })
}

/// Anomalies in `values` (one per bucket, None without data). Buckets before `eval_from` only feed
/// the baseline; anomalous buckets are left out of later baselines so a long anomaly stays one.
pub(crate) fn detect(values: &[Option<f64>], eval_from: usize, rule: Rule) -> Detection {
    let mut bands = vec![None; values.len()];
    let mut anomalous = vec![false; values.len()];
    let mut normal: Vec<f64> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let Some(v) = *v else { continue };
        let b = if i >= eval_from { band(&normal[normal.len().saturating_sub(BASELINE_BUCKETS)..], rule) } else { None };
        bands[i] = b;
        match b {
            Some(b) if v > b.upper => anomalous[i] = true,
            _ => normal.push(v),
        }
    }
    // Anomalous buckets with at most one bucket between them form one episode, kept when it has
    // MIN_ANOMALOUS of them in a row. Each candidate tracks (episode, current run, longest run).
    let mut episodes = Vec::new();
    let mut current: Option<(Episode, usize, usize)> = None;
    let close = |c: Option<(Episode, usize, usize)>, out: &mut Vec<Episode>| {
        if let Some((e, _, _)) = c.filter(|(_, _, longest)| *longest >= MIN_ANOMALOUS) {
            out.push(e);
        }
    };
    for (i, flagged) in anomalous.iter().enumerate() {
        if !flagged {
            continue;
        }
        let v = values[i].unwrap_or_default();
        match current.as_mut() {
            Some((e, run, longest)) if i - e.end <= 2 => {
                *run = if i - e.end == 1 { *run + 1 } else { 1 };
                *longest = (*longest).max(*run);
                e.end = i;
                e.peak = e.peak.max(v);
            }
            _ => {
                close(current.take(), &mut episodes);
                let baseline = bands[i].map(|b| b.median).unwrap_or_default();
                current = Some((Episode { start: i, end: i, peak: v, baseline }, 1, 1));
            }
        }
    }
    close(current, &mut episodes);
    Detection { bands, episodes }
}

/// An episode's status: resolved once a normal bucket follows it, ongoing when it reaches the last
/// bucket (or the one before, a bucket may come late), expired when the series stopped.
pub(crate) fn episode_status(values: &[Option<f64>], episode: &Episode) -> WatchdogAlertStatus {
    if values[episode.end + 1..].iter().any(Option::is_some) {
        WatchdogAlertStatus::Resolved
    } else if episode.end + 2 >= values.len() {
        WatchdogAlertStatus::Ongoing
    } else {
        WatchdogAlertStatus::Expired
    }
}

/// The analysed buckets for `now`: complete buckets only, the baseline before them.
pub(crate) fn analysis_window(now_ms: i64) -> (i64, i64) {
    let to = now_ms.div_euclid(BUCKET_MS) * BUCKET_MS;
    (to - (WINDOW_BUCKETS + BASELINE_BUCKETS) as i64 * BUCKET_MS, to)
}

/// Values per key aligned to the `n` buckets from `from`; points outside them and non-finite values are dropped.
pub(crate) fn align(points: impl IntoIterator<Item = (String, i64, Option<f64>)>, from: i64, n: usize) -> BTreeMap<String, Vec<Option<f64>>> {
    let mut out: BTreeMap<String, Vec<Option<f64>>> = BTreeMap::new();
    for (key, t, v) in points {
        let i = (t - from).div_euclid(BUCKET_MS);
        if i < 0 || i as usize >= n {
            continue;
        }
        out.entry(key).or_insert_with(|| vec![None; n])[i as usize] = v.filter(|v| v.is_finite());
    }
    out
}

/// One analysed series: a type on a service or host.
pub(crate) struct WatchdogSeries {
    pub alert_type: WatchdogAlertType,
    /// The service or host.
    pub key: String,
    /// Host metrics in the Datadog Agent's names (else OpenTelemetry's).
    pub datadog: bool,
    pub values: Vec<Option<f64>>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct WatchdogPoint {
    /// Bucket start, epoch ms.
    pub t: i64,
    pub value: Option<f64>,
    /// The expected range (null where there is no baseline).
    pub lower: Option<f64>,
    pub upper: Option<f64>,
}

/// A monitor that would catch this again: what the create form is prefilled with.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchdogMonitor {
    /// `metric`, `apm` or `log`.
    pub monitor_type: String,
    pub query: String,
    pub name: String,
    /// The top of the expected range when the anomaly started.
    pub critical: f64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WatchdogAlert {
    pub id: String,
    pub category: WatchdogCategory,
    pub alert_type: WatchdogAlertType,
    pub status: WatchdogAlertStatus,
    pub title: String,
    /// What is affected, as `key:value` tags.
    pub scope: Vec<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    /// The team owning the service (service metadata), if any.
    pub team: Option<String>,
    /// `percent`, `ms` or `logs` (per bucket).
    pub unit: String,
    /// Start of the first and end of the last anomalous bucket, epoch ms.
    pub start: i64,
    pub end: i64,
    /// The baseline median when it started, and the highest value.
    pub baseline: f64,
    pub peak: f64,
    /// The series around the anomaly, with its expected range.
    pub points: Vec<WatchdogPoint>,
    pub monitor: WatchdogMonitor,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WatchdogAlerts {
    /// Ongoing first, then the most recent.
    pub alerts: Vec<WatchdogAlert>,
    /// The analysed period (after the baseline), epoch ms.
    pub from: i64,
    pub to: i64,
    pub bucket_ms: i64,
    /// Services and hosts watched.
    pub services: u64,
    pub hosts: u64,
}

fn round(v: f64, digits: i32) -> f64 {
    let f = 10f64.powi(digits);
    (v * f).round() / f
}

/// The monitor for an alert type on a service or host, alerting above `critical`.
fn monitor(alert_type: WatchdogAlertType, key: &str, datadog: bool, critical: f64) -> WatchdogMonitor {
    use WatchdogAlertType::*;
    let (monitor_type, query) = match alert_type {
        ErrorRate => ("apm", format!("avg:trace.error_rate{{service:{key}}}")),
        Latency => ("apm", format!("avg:trace.latency.p95{{service:{key}}}")),
        ErrorLogs => ("log", format!("logs(\"service:{key} status:error\") by {{service}}")),
        Cpu if datadog => ("metric", format!("100 - avg:system.cpu.idle{{host:{key}}}")),
        Cpu => ("metric", format!("100 - avg:system.cpu.utilization{{host:{key},state:idle}} * 100")),
        Memory if datadog => ("metric", format!("100 - avg:system.mem.pct_usable{{host:{key}}} * 100")),
        Memory => ("metric", format!("avg:system.memory.utilization{{host:{key},state:used}} * 100")),
        Disk if datadog => ("metric", format!("max:system.disk.in_use{{host:{key}}} * 100")),
        Disk => ("metric", format!("max:system.filesystem.utilization{{host:{key}}} * 100")),
    };
    let critical = if alert_type == ErrorRate { round(critical, 1) } else { critical.ceil() };
    WatchdogMonitor { monitor_type: monitor_type.into(), query, name: format!("{} is high on {key}", alert_type.what().1), critical }
}

/// Alerts from analysed series whose buckets start at `from`; `teams` maps services to their team.
pub(crate) fn build_alerts(series: &[WatchdogSeries], from: i64, teams: &HashMap<String, String>) -> Vec<WatchdogAlert> {
    let mut out = Vec::new();
    for s in series {
        let d = detect(&s.values, BASELINE_BUCKETS, s.alert_type.rule());
        let on_host = s.alert_type.category() == WatchdogCategory::Infrastructure;
        let scope = format!("{}:{}", if on_host { "host" } else { "service" }, s.key);
        for e in &d.episodes {
            let lo = e.start.saturating_sub(CONTEXT_BUCKETS);
            let hi = (e.end + CONTEXT_BUCKETS).min(s.values.len() - 1);
            let points = (lo..=hi)
                .map(|i| WatchdogPoint {
                    t: from + i as i64 * BUCKET_MS,
                    value: s.values[i].map(|v| round(v, 2)),
                    lower: d.bands[i].map(|b| round(b.lower, 2)),
                    upper: d.bands[i].map(|b| round(b.upper, 2)),
                })
                .collect();
            let start = from + e.start as i64 * BUCKET_MS;
            let critical = d.bands[e.start].map_or(e.peak, |b| b.upper);
            out.push(WatchdogAlert {
                id: format!("{}:{scope}:{start}", s.alert_type.slug()),
                category: s.alert_type.category(),
                alert_type: s.alert_type,
                status: episode_status(&s.values, e),
                title: format!("{} on {scope}", s.alert_type.what().0),
                scope: vec![scope.clone()],
                service: (!on_host).then(|| s.key.clone()),
                host: on_host.then(|| s.key.clone()),
                team: if on_host { None } else { teams.get(&s.key).cloned() },
                unit: s.alert_type.unit().into(),
                start,
                end: from + (e.end as i64 + 1) * BUCKET_MS,
                baseline: round(e.baseline, 2),
                peak: round(e.peak, 2),
                points,
                monitor: monitor(s.alert_type, &s.key, s.datadog, critical),
            });
        }
    }
    let ongoing = |a: &WatchdogAlert| a.status == WatchdogAlertStatus::Ongoing;
    out.sort_by(|a, b| ongoing(b).cmp(&ongoing(a)).then(b.start.cmp(&a.start)).then_with(|| a.id.cmp(&b.id)));
    out.truncate(MAX_ALERTS);
    out
}

#[derive(clickhouse::Row, Deserialize)]
struct ApmBucket {
    service: String,
    t: i64,
    hits: u64,
    errors: u64,
    p95: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct LogBucket {
    service: String,
    t: i64,
    errors: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct HostBucket {
    host: String,
    t: i64,
    idle: Option<f64>,
    mem: Option<f64>,
    disk: Option<f64>,
    datadog: u8,
}

/// Anomalies Watchdog found over the past day: APM error rate and latency per service, error logs
/// per service (only the logs the reader may see) and host CPU, memory and disk usage.
#[utoipa::path(get, path = "/api/v1/watchdog/alerts", operation_id = "watchdog_alerts", responses((status = 200, body = WatchdogAlerts)), tag = "monitors")]
pub async fn alerts(State(st): State<AppState>, OrgId(org): OrgId, access: LogAccess) -> ApiResult<WatchdogAlerts> {
    let (from, to) = analysis_window(chrono::Utc::now().timestamp_millis());
    let n = WINDOW_BUCKETS + BASELINE_BUCKETS;
    let ch = st.ch(&org);
    let apm = ch
        .query(&format!(
            "SELECT toString(ServiceName) AS service, {t} AS t, count() AS hits, countIf(StatusCode = 'Error') AS errors, quantile(0.95)(Duration) / 1e6 AS p95 \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {ENTRY} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp < fromUnixTimestamp64Milli(?) \
             AND ServiceName IN (SELECT ServiceName FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {ENTRY} \
               AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp < fromUnixTimestamp64Milli(?) GROUP BY ServiceName ORDER BY count() DESC LIMIT ?) \
             GROUP BY service, t",
            t = BUCKET_T.replace("{col}", "Timestamp"),
        ))
        .bind(from)
        .bind(to)
        .bind(from)
        .bind(to)
        .bind(MAX_SERVICES)
        .fetch_all::<ApmBucket>();
    // Through the reader's log access, like every log read.
    let s = scope(&org, &access, TimeRange { from, to: to - 1 }, &[], None);
    let logs = bind(
        bind(
            ch.query(&format!(
                "SELECT toString(service) AS service, {t} AS t, countIf(status = 'error') AS errors FROM {table} WHERE {clause} \
                 AND service IN (SELECT service FROM {table} WHERE {clause} AND status = 'error' GROUP BY service ORDER BY count() DESC LIMIT {MAX_SERVICES}) GROUP BY service, t",
                t = BUCKET_T.replace("{col}", "timestamp"),
                table = s.table,
                clause = s.clause,
            )),
            &s.params,
        ),
        &s.params,
    )
    .fetch_all::<LogBucket>();
    let hosts = with_window(ch.clone(), from, to - 1)
        .query(&format!(
            // Datadog Agents send idle CPU in percent, usable memory and disk use as shares;
            // OpenTelemetry hostmetrics utilization by state, as shares.
            "SELECT ResourceAttributes['host.name'] AS host, {t} AS t, \
             avgOrNullIf(if(MetricName = 'system.cpu.idle', Value / 100, Value), MetricName = 'system.cpu.idle' OR (MetricName = 'system.cpu.utilization' AND Attributes['state'] = 'idle')) AS idle, \
             avgOrNullIf(if(MetricName = 'system.mem.pct_usable', 1 - Value, Value), MetricName = 'system.mem.pct_usable' OR (MetricName = 'system.memory.utilization' AND Attributes['state'] = 'used')) AS mem, \
             maxOrNullIf(Value, MetricName IN ('system.disk.in_use', 'system.filesystem.utilization')) AS disk, \
             toUInt8(countIf(MetricName IN ('system.cpu.idle', 'system.mem.pct_usable', 'system.disk.in_use')) > 0) AS datadog \
             FROM {src} WHERE OrgId = {{oc_org:String}} AND host != '' GROUP BY host, t ORDER BY host, t LIMIT ?",
            t = BUCKET_T.replace("{col}", "TimeUnix"),
            src = points_subquery("gauge", HOST_METRICS),
        ))
        .bind((MAX_HOSTS * n) as u64)
        .fetch_all::<HostBucket>();
    let teams = sqlx::query_as::<_, (String, String)>("SELECT service, team FROM service_metadata WHERE org_id = $1 AND team <> ''").bind(&org).fetch_all(&st.postgres);
    let (apm, logs, hosts, teams) =
        tokio::try_join!(async { apm.await.map_err(ApiError::from) }, async { logs.await.map_err(ApiError::from) }, async { hosts.await.map_err(ApiError::from) }, async {
            teams.await.map_err(ApiError::from)
        })?;

    let datadog: HashMap<String, bool> = hosts.iter().map(|h| (h.host.clone(), h.datadog > 0)).collect();
    let judged = |b: &ApmBucket| b.hits >= MIN_HITS;
    let aligned = [
        (WatchdogAlertType::ErrorRate, align(apm.iter().map(|b| (b.service.clone(), b.t, judged(b).then(|| b.errors as f64 * 100.0 / b.hits as f64))), from, n)),
        (WatchdogAlertType::Latency, align(apm.iter().map(|b| (b.service.clone(), b.t, judged(b).then_some(b.p95))), from, n)),
        (WatchdogAlertType::ErrorLogs, align(logs.iter().map(|b| (b.service.clone(), b.t, Some(b.errors as f64))), from, n)),
        (WatchdogAlertType::Cpu, align(hosts.iter().map(|h| (h.host.clone(), h.t, h.idle.map(|i| (1.0 - i) * 100.0))), from, n)),
        (WatchdogAlertType::Memory, align(hosts.iter().map(|h| (h.host.clone(), h.t, h.mem.map(|m| m * 100.0))), from, n)),
        (WatchdogAlertType::Disk, align(hosts.iter().map(|h| (h.host.clone(), h.t, h.disk.map(|d| d * 100.0))), from, n)),
    ];
    let series: Vec<WatchdogSeries> = aligned
        .into_iter()
        .flat_map(|(alert_type, by_key)| by_key.into_iter().map(move |(key, values)| (alert_type, key, values)))
        .map(|(alert_type, key, values)| WatchdogSeries { alert_type, datadog: datadog.get(&key).copied().unwrap_or(false), key, values })
        .collect();

    let mut services: Vec<&str> = apm.iter().map(|b| b.service.as_str()).chain(logs.iter().map(|b| b.service.as_str())).collect();
    services.sort_unstable();
    services.dedup();
    let teams: HashMap<String, String> = teams.into_iter().collect();
    Ok(Json(WatchdogAlerts {
        alerts: build_alerts(&series, from, &teams),
        from: from + BASELINE_BUCKETS as i64 * BUCKET_MS,
        to,
        bucket_ms: BUCKET_MS,
        services: services.len() as u64,
        hosts: datadog.len() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: Rule = Rule { min_delta: 1.0, min_ratio: 1.0 };

    /// `n` noisy values around `level`.
    fn steady(n: usize, level: f64) -> Vec<Option<f64>> {
        (0..n).map(|i| Some(level + [0.0, 0.2, -0.2, 0.1, -0.1][i % 5])).collect()
    }

    #[test]
    fn band_is_median_plus_robust_deviations_with_a_floor() {
        let b = band(&[10.0, 11.0, 9.0, 10.0, 12.0, 8.0, 10.0, 10.0, 10.0, 11.0, 9.0, 10.0], FLAT).unwrap();
        assert_eq!(b.median, 10.0);
        // MAD = 0.5 → σ ≈ 0.741 → K·σ ≈ 2.97.
        assert!((b.upper - 12.965).abs() < 0.01, "{b:?}");
        assert!((b.lower - 7.035).abs() < 0.01, "{b:?}");
        let flat = band(&[5.0; 12], Rule { min_delta: 2.0, min_ratio: 1.0 }).unwrap();
        assert_eq!((flat.upper, flat.lower), (7.0, 5.0), "no spread: the rule's delta is the margin above");
        let ratio = band(&[100.0; 12], Rule { min_delta: 2.0, min_ratio: 1.5 }).unwrap();
        assert_eq!(ratio.upper, 150.0);
        assert!(band(&[1.0; 11], FLAT).is_none(), "too little baseline");
        assert_eq!(band(&[0.5; 12], Rule { min_delta: 2.0, min_ratio: 1.0 }).unwrap().lower, 0.5);
    }

    #[test]
    fn a_spike_is_one_episode_and_resolves() {
        let mut v = steady(40, 1.0);
        v.extend([Some(9.0), Some(10.0), Some(12.0)]);
        v.extend(steady(10, 1.0));
        let d = detect(&v, 24, FLAT);
        assert_eq!(d.episodes.len(), 1);
        let e = d.episodes[0];
        assert_eq!((e.start, e.end, e.peak), (40, 42, 12.0));
        assert!((e.baseline - 1.0).abs() < 0.2);
        assert_eq!(episode_status(&v, &e), WatchdogAlertStatus::Resolved);
        assert!(d.bands[..24].iter().all(Option::is_none), "the baseline period isn't judged");
        assert!(d.bands[30].is_some());
    }

    #[test]
    fn single_blips_and_quiet_series_raise_nothing() {
        let mut v = steady(40, 1.0);
        v.push(Some(50.0));
        v.extend(steady(10, 1.0));
        assert!(detect(&v, 24, FLAT).episodes.is_empty(), "one bucket isn't enough");
        assert!(detect(&steady(100, 3.0), 24, FLAT).episodes.is_empty());
        // Isolated anomalous buckets every other bucket (one error in a few requests) aren't an episode.
        let mut flicker = steady(30, 1.0);
        flicker.extend([Some(9.0), Some(1.0), Some(9.0), Some(1.0), Some(9.0), Some(1.0)]);
        assert!(detect(&flicker, 24, FLAT).episodes.is_empty(), "an episode needs two anomalous buckets in a row");
    }

    #[test]
    fn a_few_errors_in_little_traffic_are_not_an_error_rate_increase() {
        // 0 or 1 error in 40 requests per bucket: 0% or 2.5%.
        let v: Vec<Option<f64>> = (0..100).map(|i| Some(if i % 5 < 2 { 2.5 } else { 0.0 })).collect();
        assert!(detect(&v, 24, WatchdogAlertType::ErrorRate.rule()).episodes.is_empty());
    }

    #[test]
    fn a_long_shift_stays_one_ongoing_episode() {
        let mut v = steady(30, 2.0);
        v.extend(steady(60, 20.0));
        let d = detect(&v, 24, FLAT);
        assert_eq!(d.episodes.len(), 1, "anomalous buckets don't become the baseline");
        assert_eq!((d.episodes[0].start, d.episodes[0].end), (30, 89));
        assert_eq!(episode_status(&v, &d.episodes[0]), WatchdogAlertStatus::Ongoing);
    }

    #[test]
    fn a_short_gap_does_not_split_an_episode() {
        let mut v = steady(30, 1.0);
        v.extend([Some(10.0), Some(10.0), Some(1.0), Some(11.0), Some(10.0)]);
        v.extend(steady(5, 1.0));
        let d = detect(&v, 24, FLAT);
        assert_eq!(d.episodes.len(), 1);
        assert_eq!((d.episodes[0].start, d.episodes[0].end), (30, 34));
    }

    #[test]
    fn a_series_that_stops_reporting_expires() {
        let mut v = steady(30, 1.0);
        v.extend([Some(10.0), Some(10.0), Some(10.0)]);
        v.extend([None; 10]);
        let d = detect(&v, 24, FLAT);
        assert_eq!(d.episodes.len(), 1);
        assert_eq!(episode_status(&v, &d.episodes[0]), WatchdogAlertStatus::Expired);
        assert!(d.bands[35].is_none(), "no band without data");
    }

    #[test]
    fn missing_buckets_do_not_count_as_baseline() {
        let mut v: Vec<Option<f64>> = vec![None; 30];
        v.extend([Some(10.0), Some(10.0), Some(10.0)]);
        assert!(detect(&v, 24, FLAT).episodes.is_empty(), "no baseline, no judgement");
    }

    #[test]
    fn types_have_categories_and_floors() {
        assert_eq!(WatchdogAlertType::ErrorRate.category(), WatchdogCategory::Apm);
        assert_eq!(WatchdogAlertType::Latency.category(), WatchdogCategory::Apm);
        assert_eq!(WatchdogAlertType::ErrorLogs.category(), WatchdogCategory::Logs);
        assert_eq!(WatchdogAlertType::Disk.category(), WatchdogCategory::Infrastructure);
        // An error rate going from 0.1% to 0.5% isn't worth an alert.
        let b = band(&[0.1; 12], WatchdogAlertType::ErrorRate.rule()).unwrap();
        assert!(b.upper > 0.5);
    }

    #[test]
    fn the_window_ends_at_the_last_complete_bucket() {
        let now = 1_700_000_123_456;
        let (from, to) = analysis_window(now);
        assert_eq!(to % BUCKET_MS, 0);
        assert!(to <= now && now - to < BUCKET_MS);
        assert_eq!(to - from, (WINDOW_BUCKETS + BASELINE_BUCKETS) as i64 * BUCKET_MS);
    }

    #[test]
    fn points_are_aligned_to_buckets() {
        let from = 1_000 * BUCKET_MS;
        let a = align(
            [
                ("order-service".to_string(), from, Some(1.0)),
                ("order-service".to_string(), from + 2 * BUCKET_MS, Some(f64::NAN)),
                ("order-service".to_string(), from + 3 * BUCKET_MS, Some(3.0)),
                ("billing".to_string(), from - BUCKET_MS, Some(9.0)),
                ("billing".to_string(), from + 4 * BUCKET_MS, Some(9.0)),
            ],
            from,
            4,
        );
        assert_eq!(a["order-service"], vec![Some(1.0), None, None, Some(3.0)]);
        assert!(!a.contains_key("billing"), "points outside the buckets are dropped");
    }

    #[test]
    fn alerts_carry_scope_team_window_and_a_monitor() {
        let from = 1_000 * BUCKET_MS;
        let mut errors = steady(60, 1.0);
        errors.extend([Some(12.0), Some(15.0), Some(14.0)]);
        let mut cpu = steady(100, 30.0);
        cpu.extend([Some(95.0), Some(97.0)]);
        cpu.extend(steady(5, 30.0));
        let series = [
            WatchdogSeries { alert_type: WatchdogAlertType::Cpu, key: "i-0a1b2c3d4e5f60001".into(), datadog: true, values: cpu },
            WatchdogSeries { alert_type: WatchdogAlertType::ErrorRate, key: "order-service".into(), datadog: false, values: errors },
            WatchdogSeries { alert_type: WatchdogAlertType::Latency, key: "billing".into(), datadog: false, values: steady(80, 120.0) },
        ];
        let teams = HashMap::from([("order-service".to_string(), "checkout".to_string())]);
        let alerts = build_alerts(&series, from, &teams);
        assert_eq!(alerts.len(), 2, "billing's latency is steady");

        let err = &alerts[0];
        assert_eq!(err.status, WatchdogAlertStatus::Ongoing, "ongoing alerts come first");
        assert_eq!(err.title, "Error rate increase on service:order-service");
        assert_eq!(err.scope, vec!["service:order-service"]);
        assert_eq!((err.service.as_deref(), err.host.as_deref(), err.team.as_deref()), (Some("order-service"), None, Some("checkout")));
        assert_eq!((err.start, err.end), (from + 60 * BUCKET_MS, from + 63 * BUCKET_MS));
        assert_eq!(err.id, format!("error_rate:service:order-service:{}", from + 60 * BUCKET_MS));
        assert_eq!(err.peak, 15.0);
        assert_eq!(err.points.first().unwrap().t, from + 36 * BUCKET_MS, "two hours before");
        assert_eq!(err.points.last().unwrap().t, from + 62 * BUCKET_MS, "up to the last bucket");
        assert!(err.points.iter().all(|p| p.upper.is_some()));
        assert_eq!(
            err.monitor,
            WatchdogMonitor { monitor_type: "apm".into(), query: "avg:trace.error_rate{service:order-service}".into(), name: "Error rate is high on order-service".into(), critical: 6.0 }
        );

        let cpu = &alerts[1];
        assert_eq!((cpu.category, cpu.status, cpu.unit.as_str()), (WatchdogCategory::Infrastructure, WatchdogAlertStatus::Resolved, "percent"));
        assert_eq!(cpu.scope, vec!["host:i-0a1b2c3d4e5f60001"]);
        assert_eq!(cpu.team, None);
        assert_eq!(cpu.monitor.monitor_type, "metric");
        assert_eq!(cpu.monitor.query, "100 - avg:system.cpu.idle{host:i-0a1b2c3d4e5f60001}");
        assert_eq!(cpu.monitor.critical, 50.0);
    }

    #[test]
    fn host_monitors_follow_the_metric_names_the_host_sends() {
        assert_eq!(monitor(WatchdogAlertType::Memory, "h", false, 80.2).query, "avg:system.memory.utilization{host:h,state:used} * 100");
        assert_eq!(monitor(WatchdogAlertType::Memory, "h", true, 80.2).query, "100 - avg:system.mem.pct_usable{host:h} * 100");
        assert_eq!(monitor(WatchdogAlertType::Disk, "h", false, 80.0).query, "max:system.filesystem.utilization{host:h} * 100");
        let logs = monitor(WatchdogAlertType::ErrorLogs, "billing", false, 31.5);
        assert_eq!((logs.monitor_type.as_str(), logs.query.as_str(), logs.critical), ("log", "logs(\"service:billing status:error\") by {service}", 32.0));
    }
}
