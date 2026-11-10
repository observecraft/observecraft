//! The GCP integration's Cloud Monitoring crawler, on the cloud crawlers' jobs (`crate::cloud::jobs`).
//!
//! - **Discovery.** Each service account has a discovery job (scope `""`): this install impersonates it
//!   and lists the active projects it can see, hourly; each becomes a job, so an organization's projects
//!   spread over the servers.
//! - **A project's run** ([`crawl`]) lists its metric descriptors, keeps those of our catalog's services
//!   that aren't turned off (and the user-defined domains turned on, `metric_namespace_configs`) and can be
//!   read, and reads each one's time series over the window since the watermark (at most 3 hours, ending 5
//!   minutes ago), several metric types at a time. Series whose monitored resource the filters exclude
//!   (`monitored_resource_configs`, `region_filter_configs`, global locations) are dropped.
//! - **Late and coarse metrics.** A one-minute metric whose ingest delay is longer than the lag is read
//!   over the same window moved back by the difference, so its points are there when read and each is
//!   read once. Metrics sampled less often (5 minutes, hourly, daily: Cloud Storage's sizes and object
//!   counts) are re-read over their last intervals on the first run, hourly and when one of their
//!   intervals completed, and only points after each series' latest published one are published
//!   (Postgres `gcp_metric_state.marks`).
//! - **Publishing.** Datadog names (`gcp.gce.instance.cpu.utilization`; cumulative counters' running
//!   total and `.delta`, distributions' `.avg`, `.samplecount`, `.sumsqdev` and some `.p95` / `.p99`,
//!   `naming::stats`), tagged with the project, the resource's and the metric's labels, the resource's
//!   user labels (Cloud Monitoring's metadata), an instance's type, a zone's region and the account's
//!   tags; instances are the `host`. OTLP JSON to Kafka, the watermark advancing after the acknowledgement.
//! - **Failed reads** (throttling, server errors) don't drop the run: what was read is published, the failed
//!   one-minute reads are kept (`gcp_metric_state.pending`, a day at most) and retried on the next runs, and
//!   the run's warning says so. A run whose every read failed is the account's issue (backoff).
//! - **Failures** are the account's issues: it can't be impersonated (Service Account Token Creator), a
//!   missing permission (named), the Cloud Monitoring API disabled in a project (retried daily).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{aligner, Descriptor, GcpApi, GcpError, Reading, Sample, Series};
use super::naming::{self, Stat};
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::cloud::naming::{otlp_json, Point};
use crate::routes::datadog_api::gcp_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "gcp";
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const LAG_MS: i64 = 5 * MINUTE_MS;
const MAX_WINDOW_MS: i64 = 3 * HOUR_MS;
/// The furthest a one-minute metric's window moves back for its ingest delay.
const MAX_SHIFT_MS: i64 = 6 * HOUR_MS;
/// Marks older than this are dropped (their series are gone).
const MARKS_KEEP_MS: i64 = 4 * DAY_MS;
/// Failed reads kept for the next runs: at most this many, for at most a day.
const MAX_PENDING: usize = 200;
const PENDING_MAX_AGE_MS: i64 = DAY_MS;
/// Metric types read at a time within a project.
const PARALLEL: usize = 8;
const POINTS_PER_RECORD: usize = 10_000;

/// A Google error as the service account's issue.
pub fn issue(e: &GcpError) -> Issue {
    match e {
        GcpError::Auth(m) => Issue { kind: "auth", message: m.clone(), missing_action: None, backoff: Backoff::Interval },
        GcpError::Forbidden { permission, message } => Issue { kind: "access_denied", message: message.clone(), missing_action: permission.clone(), backoff: Backoff::Interval },
        GcpError::Disabled(m) => Issue { kind: "disabled", message: m.clone(), missing_action: None, backoff: Backoff::Day },
        GcpError::Throttled(m) => Issue { kind: "throttled", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
        GcpError::Other(m) => Issue { kind: "other", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
    }
}

fn split_tag(t: &str) -> (String, String) {
    let (k, v) = t.split_once(':').unwrap_or((t, ""));
    (k.to_string(), v.to_string())
}

/// A one-minute metric's read that failed, retried on the next runs: its interval and what is published from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub metric_type: String,
    pub from: i64,
    pub to: i64,
    pub after: Option<i64>,
}

/// A project job's state beside its watermark (Postgres `gcp_metric_state`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The latest published point of each coarse series ([`mark_key`] → epoch ms).
    #[serde(default)]
    pub marks: HashMap<String, i64>,
    #[serde(default)]
    pub pending: Vec<Pending>,
}

/// A run's outcome and, when it collected, the state to keep.
#[derive(Debug, PartialEq)]
pub struct Crawl {
    pub outcome: Outcome,
    pub state: Option<State>,
}

impl From<Outcome> for Crawl {
    fn from(outcome: Outcome) -> Self {
        Crawl { outcome, state: None }
    }
}

/// A series' key in [`State::marks`]: metric, resource type, resource and metric labels.
pub fn mark_key(metric_type: &str, s: &Series) -> String {
    let join = |l: &[(String, String)]| l.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",");
    format!("{metric_type}|{}|{}|{}", s.resource_type, join(&s.resource_labels), join(&s.metric_labels))
}

/// Whether a metric is sampled less often than once a minute (it is then re-read with marks).
fn coarse(d: &Descriptor) -> bool {
    d.sample_period_ms > MINUTE_MS
}

/// How far a one-minute metric's window moves back: its ingest delay beyond the lag, in whole minutes.
fn shift_ms(d: &Descriptor) -> i64 {
    let beyond = (d.ingest_delay_ms - LAG_MS).max(0);
    ((beyond + MINUTE_MS - 1) / MINUTE_MS * MINUTE_MS).min(MAX_SHIFT_MS)
}

/// How far back a coarse metric is re-read: 3 of its intervals and its ingest delay, at least an hour, at most 3 days.
fn lookback_ms(d: &Descriptor) -> i64 {
    (3 * d.sample_period_ms + d.ingest_delay_ms).clamp(HOUR_MS, 3 * DAY_MS)
}

/// A metric's read in a run: the interval asked for (cumulative counters reach back for their previous
/// point) and what is published from it: points after `after`, or after each series' mark.
struct Read<'a> {
    d: &'a Descriptor,
    from: i64,
    to: i64,
    after: Option<i64>,
}

fn plan(d: &Descriptor, start: i64, end: i64, hourly: bool) -> Option<Read<'_>> {
    let raw_counter = d.kind == "CUMULATIVE" && d.value_type != "DISTRIBUTION";
    if coarse(d) {
        let due = hourly || start / d.sample_period_ms != end / d.sample_period_ms;
        return due.then(|| Read { d, from: end - lookback_ms(d), to: end, after: None });
    }
    let (from, to) = (start - shift_ms(d), end - shift_ms(d));
    let baseline = if raw_counter { 2 * d.sample_period_ms.max(MINUTE_MS) } else { 0 };
    Some(Read { d, from: from - baseline, to, after: Some(from) })
}

/// A series' tags: the account's, the project, the resource's and the metric's labels, the resource's
/// user labels, an instance's type (`instance-type`) and a zone's region.
fn series_tags(account_tags: &[(String, String)], project: &str, s: &Series) -> BTreeMap<String, String> {
    let mut tags: BTreeMap<String, String> = account_tags.iter().cloned().collect();
    tags.insert("project_id".into(), project.into());
    tags.extend(s.resource_labels.iter().cloned());
    tags.extend(s.metric_labels.iter().cloned());
    for (k, v) in &s.user_labels {
        tags.entry(k.clone()).or_insert_with(|| v.clone());
    }
    if s.resource_type == "gce_instance" {
        if let Some((_, t)) = s.system_labels.iter().find(|(k, _)| k == "machine_type") {
            tags.insert("instance-type".into(), t.clone());
        }
    }
    if let Some(region) = tags.get("zone").and_then(|z| z.rsplit_once('-')).map(|(r, _)| r.to_string()) {
        tags.entry("region".into()).or_insert(region);
    }
    tags
}

/// The value one name of a point carries (`prev`: the series' previous point, for a counter's delta).
fn stat_value(stat: Stat, p: &Sample, prev: Option<&Sample>) -> Option<f64> {
    match (&p.value, stat) {
        (Reading::Number(v), Stat::Value) => Some(*v),
        (Reading::Number(v), Stat::Delta) => match prev.map(|q| (q.start_ms, &q.value)) {
            // Reset (a new start, or a lower total): the total since the reset.
            Some((start, Reading::Number(before))) => Some(if start == p.start_ms && v >= before { v - before } else { *v }),
            _ => None,
        },
        (Reading::Distribution(d), Stat::SampleCount) => Some(d.count),
        (Reading::Distribution(d), _) if d.count <= 0.0 => None,
        (Reading::Distribution(d), Stat::Avg) => Some(d.mean),
        (Reading::Distribution(d), Stat::SumSqDev) => Some(d.sumsqdev),
        (Reading::Distribution(d), Stat::P95) => d.percentile(0.95),
        (Reading::Distribution(d), Stat::P99) => d.percentile(0.99),
        _ => None,
    }
}

/// A read's series as points; with no `after`, only points after each series' mark (the marks advance).
/// The number of series published.
fn to_points(read: &Read, project: &str, account_tags: &[(String, String)], config: &Value, series: Vec<Series>, marks: &mut HashMap<String, i64>, out: &mut Vec<Point>) -> usize {
    let d = read.d;
    let (name, stats) = (naming::metric_name(&d.metric_type), naming::stats(&d.metric_type, &d.kind, &d.value_type));
    let mut published = 0;
    for mut s in series {
        let tags = series_tags(account_tags, project, &s);
        let flat: Vec<String> = tags.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        if !naming::resource_selected(config, &s.resource_type, &flat) || !naming::location_selected(config, &tags) {
            continue;
        }
        let key = read.after.is_none().then(|| mark_key(&d.metric_type, &s));
        let after = read.after.or_else(|| key.as_ref().and_then(|k| marks.get(k).copied())).unwrap_or(i64::MIN);
        let host = (s.resource_type == "gce_instance").then(|| tags.get("instance_id").cloned()).flatten();
        let tags: Vec<(String, String)> = tags.into_iter().collect();
        // Google lists the newest point first.
        s.points.sort_by_key(|p| p.time_ms);
        let (mut newest, before) = (None, out.len());
        for (i, p) in s.points.iter().enumerate().filter(|(_, p)| p.time_ms > after) {
            let prev = i.checked_sub(1).map(|j| &s.points[j]);
            for &stat in &stats {
                if let Some(value) = stat_value(stat, p, prev) {
                    out.push(Point { name: format!("{name}{}", stat.suffix()), host: host.clone(), tags: tags.clone(), time_ms: p.time_ms, value });
                }
            }
            newest = Some(p.time_ms);
        }
        if let (Some(k), Some(t)) = (key, newest) {
            marks.insert(k, t);
        }
        published += usize::from(out.len() > before);
    }
    published
}

/// Reads and publishes one project's window.
#[allow(clippy::too_many_arguments)]
pub async fn crawl(api: &dyn GcpApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, service_account: &str, config: &Value, project: &str, collected_until: Option<i64>, state: &State, now_ms: i64, interval_ms: i64) -> Crawl {
    let end = (now_ms - LAG_MS) / MINUTE_MS * MINUTE_MS;
    let start = collected_until.unwrap_or(end - interval_ms).max(end - MAX_WINDOW_MS);
    if end <= start {
        return Outcome::Idle.into();
    }
    let descriptors: Vec<Descriptor> = match api.descriptors(service_account, project).await {
        Ok(d) => d.into_iter().filter(|d| naming::collected(config, &d.metric_type) && aligner(d).is_some() && !naming::stats(&d.metric_type, &d.kind, &d.value_type).is_empty()).collect(),
        Err(e) => return Outcome::Failed(issue(&e)).into(),
    };
    let account_tags: Vec<(String, String)> = config["account_tags"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(split_tag).collect()).unwrap_or_default();
    let hourly = collected_until.is_none() || start / HOUR_MS != end / HOUR_MS;
    // The failed reads of earlier runs (still within a day, of metrics still collected) first.
    let by_type: HashMap<&str, &Descriptor> = descriptors.iter().map(|d| (d.metric_type.as_str(), d)).collect();
    let retried = state.pending.iter().filter(|p| p.to > end - PENDING_MAX_AGE_MS).filter_map(|p| by_type.get(p.metric_type.as_str()).map(|d| Read { d, from: p.from, to: p.to, after: p.after }));
    let reads: Vec<Read> = retried.chain(descriptors.iter().filter_map(|d| plan(d, start, end, hourly))).collect();
    // The calls are built first: a stream combinator's closure over borrowed descriptors isn't Send-general.
    let calls: Vec<_> = reads.iter().enumerate().map(|(i, r)| async move { (i, api.time_series(service_account, project, r.d, r.from, r.to).await) }).collect();
    let results: Vec<(usize, Result<Vec<Series>, GcpError>)> = stream::iter(calls).buffer_unordered(PARALLEL).collect().await;
    let mut marks: HashMap<String, i64> = state.marks.iter().filter(|(_, &at)| at >= end - MARKS_KEEP_MS).map(|(k, v)| (k.clone(), *v)).collect();
    let (mut points, mut metrics, mut ok, mut failure, mut pending) = (vec![], 0, 0, None, vec![]);
    for (i, result) in results {
        match result {
            Ok(series) => {
                ok += 1;
                metrics += to_points(&reads[i], project, &account_tags, config, series, &mut marks, &mut points);
            }
            Err(e @ (GcpError::Auth(_) | GcpError::Forbidden { .. } | GcpError::Disabled(_))) => return Outcome::Failed(issue(&e)).into(),
            Err(e) => {
                failure.get_or_insert_with(|| issue(&e));
                // Coarse metrics are re-read over their last intervals anyway.
                let r = &reads[i];
                if !coarse(r.d) {
                    pending.push((i, Pending { metric_type: r.d.metric_type.clone(), from: r.from, to: r.to, after: r.after }));
                }
            }
        }
    }
    let failed = reads.len() - ok;
    if ok == 0 && failed > 0 {
        // Nothing was read: the watermark stays and the next run reads the window again.
        return Outcome::Failed(failure.unwrap_or_else(|| issue(&GcpError::Other("every read failed".into())))).into();
    }
    for record in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, otlp_json(PROVIDER, record)).await {
            return Outcome::Failed(Issue { kind: "other", message: format!("publishing to Kafka failed: {e}"), missing_action: None, backoff: Backoff::Exponential }).into();
        }
    }
    let warning = failure.map(|f| Issue { message: format!("{failed} of {} Cloud Monitoring reads failed and are retried on the next runs: {}", reads.len(), f.message), ..f });
    // In the order read, the oldest dropped first.
    pending.sort_by_key(|(i, _)| *i);
    let mut pending: Vec<Pending> = pending.into_iter().map(|(_, p)| p).collect();
    if pending.len() > MAX_PENDING {
        pending.drain(..pending.len() - MAX_PENDING);
    }
    Crawl { outcome: Outcome::Collected { end_ms: end, metrics, points: points.len(), warning }, state: Some(State { marks, pending }) }
}

async fn load_state(st: &AppState, key: &JobKey) -> Result<State, sqlx::Error> {
    let row: Option<(Value, Value)> = sqlx::query_as("SELECT marks, pending FROM gcp_metric_state WHERE org_id = $1 AND account_id = $2::uuid AND project = $3")
        .bind(&key.org_id)
        .bind(&key.account_id)
        .bind(&key.scope)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(|(marks, pending)| State { marks: serde_json::from_value(marks).unwrap_or_default(), pending: serde_json::from_value(pending).unwrap_or_default() }).unwrap_or_default())
}

/// Keeps a project job's state (once its points are published).
async fn save_state(st: &AppState, key: &JobKey, state: &State) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO gcp_metric_state (org_id, account_id, project, marks, pending) VALUES ($1, $2::uuid, $3, $4, $5) \
         ON CONFLICT (org_id, account_id, project) DO UPDATE SET marks = EXCLUDED.marks, pending = EXCLUDED.pending, updated_at = now()",
    )
    .bind(&key.org_id)
    .bind(&key.account_id)
    .bind(&key.scope)
    .bind(serde_json::to_value(&state.marks).unwrap_or_default())
    .bind(serde_json::to_value(&state.pending).unwrap_or_default())
    .execute(&st.postgres)
    .await?;
    Ok(())
}

/// The GCP crawler: Cloud Monitoring through `api`, points to `sink`.
pub struct GcpCrawler {
    pub api: Arc<dyn GcpApi>,
    pub sink: Arc<dyn MetricSink>,
    pub interval_secs: i64,
}

#[async_trait::async_trait]
impl Crawler for GcpCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One discovery job per service account.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT a.org_id, a.id::text FROM gcp_accounts a JOIN orgs o ON o.id = a.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, String::new())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(account) = accounts::get_row(st, &job.org_id, &job.account_id).await? else { return Ok(Outcome::Idle) };
        if job.scope.is_empty() {
            return Ok(match self.api.projects(&account.client_email).await {
                Ok(projects) => Outcome::Discovered(projects),
                Err(e) => Outcome::Failed(issue(&e)),
            });
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        let state = load_state(st, job).await?;
        let run = crawl(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &account.client_email, &account.config, &job.scope, job.collected_until, &state, now_ms, self.interval_secs * 1000).await;
        if let (Outcome::Collected { .. }, Some(state)) = (&run.outcome, &run.state) {
            save_state(st, job, state).await?;
        }
        Ok(run.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::{published, FakeSink};
    use serde_json::json;
    use std::collections::HashSet;
    use std::sync::Mutex as StdMutex;

    use super::super::api::Distribution;

    #[derive(Default)]
    struct FakeGcp {
        descriptors: Vec<Descriptor>,
        /// Each metric type's series; their points are returned within the asked interval.
        series: HashMap<String, Vec<Series>>,
        fail: Option<GcpError>,
        /// Metric types whose reads fail with this error.
        failing: StdMutex<HashMap<String, GcpError>>,
        /// The intervals asked for: metric type, start, end.
        calls: StdMutex<Vec<(String, i64, i64)>>,
    }

    #[async_trait::async_trait]
    impl GcpApi for FakeGcp {
        async fn projects(&self, _sa: &str) -> Result<Vec<String>, GcpError> {
            Ok(vec!["example-storefront".into()])
        }
        async fn descriptors(&self, _sa: &str, _p: &str) -> Result<Vec<Descriptor>, GcpError> {
            Ok(self.descriptors.clone())
        }
        async fn time_series(&self, _sa: &str, _p: &str, d: &Descriptor, start: i64, end: i64) -> Result<Vec<Series>, GcpError> {
            if let Some(e) = self.fail.clone().or_else(|| self.failing.lock().unwrap().get(&d.metric_type).cloned()) {
                return Err(e);
            }
            self.calls.lock().unwrap().push((d.metric_type.clone(), start, end));
            Ok(self.series.get(&d.metric_type).cloned().unwrap_or_default().into_iter().map(|mut s| {
                s.points.retain(|p| p.time_ms > start && p.time_ms <= end);
                s.points.reverse();
                s
            }).collect())
        }
    }

    fn d(t: &str, kind: &str, value: &str) -> Descriptor {
        Descriptor { metric_type: t.into(), kind: kind.into(), value_type: value.into(), sample_period_ms: MINUTE_MS, ingest_delay_ms: 0 }
    }

    fn num(t: i64, v: f64) -> Sample {
        Sample { start_ms: t, time_ms: t, value: Reading::Number(v) }
    }

    /// One point a minute over the last 6 hours before `NOW`.
    fn minutes(v: f64) -> Vec<Sample> {
        (0..360).map(|i| num(END - i * MINUTE_MS, v)).collect()
    }

    fn instance(id: &str, env: &str) -> Series {
        Series {
            resource_type: "gce_instance".into(),
            resource_labels: vec![("instance_id".into(), id.into()), ("zone".into(), "europe-west1-b".into())],
            metric_labels: vec![("instance_name".into(), format!("web-{id}"))],
            user_labels: vec![("env".into(), env.into()), ("team".into(), "storefront".into())],
            system_labels: vec![("machine_type".into(), "e2-standard-4".into())],
            points: minutes(0.5),
        }
    }

    const CPU: &str = "compute.googleapis.com/instance/cpu/utilization";
    const SQL: &str = "cloudsql.googleapis.com/database/cpu/utilization";
    const NOW: i64 = 1_700_000_000_000;
    const END: i64 = (NOW - LAG_MS) / MINUTE_MS * MINUTE_MS;

    async fn run(gcp: &FakeGcp, config: &Value, collected_until: Option<i64>, state: &State, now: i64) -> (Crawl, FakeSink) {
        let sink = FakeSink::default();
        let crawl = crawl(gcp, &sink, &[], "acme", "sa@example-storefront.iam.gserviceaccount.com", config, "example-storefront", collected_until, state, now, 600_000).await;
        (crawl, sink)
    }

    /// Every published point: name, time, value, tags.
    fn values(sink: &FakeSink) -> Vec<(String, i64, f64, HashMap<String, String>)> {
        let mut out = vec![];
        for (_, _, doc) in sink.records.lock().unwrap().iter() {
            for rm in doc["resourceMetrics"].as_array().unwrap() {
                for m in rm["scopeMetrics"][0]["metrics"].as_array().unwrap() {
                    for dp in m["gauge"]["dataPoints"].as_array().unwrap() {
                        let tags = dp["attributes"].as_array().unwrap().iter().map(|a| (a["key"].as_str().unwrap().to_string(), a["value"]["stringValue"].as_str().unwrap().to_string())).collect();
                        let t = dp["timeUnixNano"].as_str().unwrap().parse::<i128>().unwrap() / 1_000_000;
                        out.push((m["name"].as_str().unwrap().to_string(), t as i64, dp["asDouble"].as_f64().unwrap(), tags));
                    }
                }
            }
        }
        out.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        out
    }

    #[tokio::test]
    async fn a_project_run_publishes_the_selected_series_named_and_tagged_as_datadog_does() {
        let sql = Series { resource_type: "cloudsql_database".into(), resource_labels: vec![("database_id".into(), "p:orders".into()), ("region".into(), "europe-west1".into())], user_labels: vec![("team".into(), "orders".into())], points: minutes(0.2), ..Default::default() };
        let gcp = FakeGcp {
            descriptors: vec![d(CPU, "GAUGE", "DOUBLE"), d(SQL, "GAUGE", "DOUBLE"), d("custom.googleapis.com/checkout/orders", "GAUGE", "DOUBLE"), d("compute.googleapis.com/instance/uptime_total", "GAUGE", "STRING")],
            series: HashMap::from([(CPU.to_string(), vec![instance("111", "prod"), instance("222", "staging")]), (SQL.to_string(), vec![sql])]),
            ..Default::default()
        };
        let config = json!({ "monitored_resource_configs": [{ "type": "gce_instance", "filters": ["env:prod"] }], "metric_namespace_configs": [], "account_tags": ["team:platform"] });
        let (crawl, sink) = run(&gcp, &config, None, &State::default(), NOW).await;
        // 10 minutes of each kept series.
        assert_eq!(crawl.outcome, Outcome::Collected { end_ms: END, metrics: 2, points: 20, warning: None });
        let points = published(&sink);
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(names, HashSet::from(["gcp.gce.instance.cpu.utilization", "gcp.cloudsql.database.cpu.utilization"]));
        let cpu = points.iter().find(|p| p.0 == "gcp.gce.instance.cpu.utilization").unwrap();
        assert_eq!(cpu.1.as_deref(), Some("111"));
        let tags: HashMap<_, _> = cpu.2.iter().cloned().collect();
        assert_eq!((tags["project_id"].as_str(), tags["zone"].as_str(), tags["region"].as_str(), tags["instance-type"].as_str()), ("example-storefront", "europe-west1-b", "europe-west1", "e2-standard-4"));
        // User labels are tags; the account's tags and the resource's own labels win.
        assert_eq!((tags["env"].as_str(), tags["team"].as_str()), ("prod", "platform"));
        // A disabled namespace is skipped; user-defined metrics are read once their namespace is turned on.
        let off = json!({ "metric_namespace_configs": [{ "id": "cloudsql", "disabled": true }, { "id": "custom", "disabled": false }] });
        run(&gcp, &off, None, &State::default(), NOW).await;
        let asked: HashSet<String> = gcp.calls.lock().unwrap().iter().map(|c| c.0.clone()).collect();
        assert!(asked.contains("custom.googleapis.com/checkout/orders"));
        let (_, sink) = run(&gcp, &off, None, &State::default(), NOW).await;
        assert!(published(&sink).iter().all(|p| !p.0.starts_with("gcp.cloudsql.")));
    }

    #[tokio::test]
    async fn region_filters_and_global_locations_drop_series() {
        let gcp = FakeGcp { descriptors: vec![d(CPU, "GAUGE", "DOUBLE"), d(SQL, "GAUGE", "DOUBLE")], series: HashMap::from([(CPU.to_string(), vec![instance("111", "prod")]), (SQL.to_string(), vec![Series { resource_type: "cloudsql_database".into(), resource_labels: vec![("region".into(), "us-central1".into())], points: minutes(1.0), ..Default::default() }])]), ..Default::default() };
        let (_, sink) = run(&gcp, &json!({ "region_filter_configs": ["europe-west1"] }), None, &State::default(), NOW).await;
        let names: HashSet<String> = published(&sink).into_iter().map(|p| p.0).collect();
        assert_eq!(names, HashSet::from(["gcp.gce.instance.cpu.utilization".to_string()]));
    }

    #[tokio::test]
    async fn cumulative_counters_carry_their_total_and_delta_and_distributions_their_statistics() {
        const RESTARTS: &str = "kubernetes.io/container/restart_count";
        const LATENCY: &str = "loadbalancing.googleapis.com/https/total_latencies";
        const ACKS: &str = "pubsub.googleapis.com/subscription/ack_latencies";
        let start = END - 600_000;
        let reset = start + 3 * MINUTE_MS;
        // A total of 4, 5, 7 since one start, then a reset: 1.
        let counter = |t: i64, since: i64, v: f64| Sample { start_ms: since, time_ms: t, value: Reading::Number(v) };
        let container = Series { resource_type: "k8s_container".into(), resource_labels: vec![("cluster_name".into(), "storefront-prod".into())], points: vec![counter(start - MINUTE_MS, 0, 4.0), counter(start + MINUTE_MS, 0, 5.0), counter(start + 2 * MINUTE_MS, 0, 7.0), counter(start + 4 * MINUTE_MS, reset, 1.0)], ..Default::default() };
        let dist = Distribution { count: 100.0, mean: 42.5, sumsqdev: 1200.0, bounds: vec![10.0, 20.0, 50.0, 100.0], counts: vec![0.0, 10.0, 40.0, 45.0, 5.0] };
        let histogram = |t: i64| Sample { start_ms: t - MINUTE_MS, time_ms: t, value: Reading::Distribution(dist.clone()) };
        let lb = Series { resource_type: "https_lb_rule".into(), resource_labels: vec![("url_map_name".into(), "storefront-url-map".into())], points: vec![histogram(END)], ..Default::default() };
        let sub = Series { resource_type: "pubsub_subscription".into(), resource_labels: vec![("subscription_id".into(), "orders".into())], points: vec![histogram(END)], ..Default::default() };
        let gcp = FakeGcp {
            descriptors: vec![d(RESTARTS, "CUMULATIVE", "INT64"), d(LATENCY, "DELTA", "DISTRIBUTION"), d(ACKS, "DELTA", "DISTRIBUTION")],
            series: HashMap::from([(RESTARTS.to_string(), vec![container]), (LATENCY.to_string(), vec![lb]), (ACKS.to_string(), vec![sub])]),
            ..Default::default()
        };
        let (_, sink) = run(&gcp, &json!({}), Some(start), &State::default(), NOW).await;
        // The counter is read from before the window for its previous point.
        assert!(gcp.calls.lock().unwrap().iter().any(|c| c.0 == RESTARTS && c.1 == start - 2 * MINUTE_MS && c.2 == END));
        let got: Vec<(String, i64, f64)> = values(&sink).into_iter().map(|(n, t, v, _)| (n, t, v)).collect();
        let of = |name: &str| got.iter().filter(|p| p.0 == name).map(|p| (p.1, p.2)).collect::<Vec<_>>();
        assert_eq!(of("gcp.gke.container.restart_count"), [(start + MINUTE_MS, 5.0), (start + 2 * MINUTE_MS, 7.0), (start + 4 * MINUTE_MS, 1.0)]);
        assert_eq!(of("gcp.gke.container.restart_count.delta"), [(start + MINUTE_MS, 1.0), (start + 2 * MINUTE_MS, 2.0), (start + 4 * MINUTE_MS, 1.0)]);
        assert_eq!(of("gcp.loadbalancing.https.total_latencies.avg"), [(END, 42.5)]);
        assert_eq!(of("gcp.loadbalancing.https.total_latencies.samplecount"), [(END, 100.0)]);
        assert_eq!(of("gcp.loadbalancing.https.total_latencies.sumsqdev"), [(END, 1200.0)]);
        assert_eq!(of("gcp.loadbalancing.https.total_latencies.p95"), [(END, 100.0)]);
        assert_eq!(of("gcp.loadbalancing.https.total_latencies.p99"), [(END, 100.0)]);
        // Pub/Sub's latencies have no percentiles in Datadog.
        let acks: HashSet<&str> = got.iter().filter(|p| p.0.starts_with("gcp.pubsub.")).map(|p| p.0.as_str()).collect();
        assert_eq!(acks, HashSet::from(["gcp.pubsub.subscription.ack_latencies.avg", "gcp.pubsub.subscription.ack_latencies.samplecount", "gcp.pubsub.subscription.ack_latencies.sumsqdev"]));
        assert!(got.iter().all(|p| !p.0.ends_with("total_latencies") && !p.0.ends_with("ack_latencies")));
    }

    #[tokio::test]
    async fn late_metrics_are_read_over_a_window_moved_back_by_their_ingest_delay() {
        const REQUESTS: &str = "storage.googleapis.com/api/request_count";
        let late = Descriptor { ingest_delay_ms: 690_000, ..d(REQUESTS, "DELTA", "INT64") };
        let bucket = Series { resource_type: "gcs_bucket".into(), resource_labels: vec![("bucket_name".into(), "example-assets".into())], points: minutes(3.0), ..Default::default() };
        let gcp = FakeGcp { descriptors: vec![late], series: HashMap::from([(REQUESTS.to_string(), vec![bucket])]), ..Default::default() };
        let start = END - 600_000;
        let (crawl, sink) = run(&gcp, &json!({}), Some(start), &State::default(), NOW).await;
        // 690 s of delay, 300 s of lag: 7 minutes back.
        assert_eq!(gcp.calls.lock().unwrap()[0], (REQUESTS.to_string(), start - 7 * MINUTE_MS, END - 7 * MINUTE_MS));
        assert!(matches!(crawl.outcome, Outcome::Collected { end_ms: END, points: 10, .. }));
        assert!(values(&sink).iter().all(|p| p.0 == "gcp.storage.api.request_count" && p.1 > start - 7 * MINUTE_MS && p.1 <= END - 7 * MINUTE_MS));
    }

    #[tokio::test]
    async fn coarse_metrics_are_re_read_over_their_last_intervals_and_each_point_is_published_once() {
        const BYTES: &str = "storage.googleapis.com/storage/v2/deleted_bytes";
        let daily = Descriptor { sample_period_ms: DAY_MS, ingest_delay_ms: 2 * HOUR_MS, ..d(BYTES, "DELTA", "INT64") };
        let day = END / DAY_MS * DAY_MS;
        let bucket = |points: Vec<Sample>| Series { resource_type: "gcs_bucket".into(), resource_labels: vec![("bucket_name".into(), "example-assets".into())], points, ..Default::default() };
        let gcp = FakeGcp { descriptors: vec![daily.clone()], series: HashMap::from([(BYTES.to_string(), vec![bucket(vec![num(day - DAY_MS, 10.0), num(day, 20.0)])])]), ..Default::default() };
        // The first run reads the last 3 days and publishes both days.
        let (crawl, sink) = run(&gcp, &json!({}), None, &State::default(), NOW).await;
        assert_eq!(gcp.calls.lock().unwrap()[0], (BYTES.to_string(), END - 3 * DAY_MS, END));
        assert_eq!(values(&sink).iter().map(|p| p.1).collect::<Vec<_>>(), [day - DAY_MS, day]);
        let state = crawl.state.unwrap();
        assert_eq!(state.marks.values().copied().collect::<Vec<_>>(), [day]);
        // Within the same hour: not read.
        let next = NOW + 2 * MINUTE_MS;
        let (_, sink) = run(&gcp, &json!({}), Some(END), &state, next).await;
        assert_eq!(gcp.calls.lock().unwrap().len(), 1);
        assert!(sink.records.lock().unwrap().is_empty());
        // The next hour: re-read, and only the day that arrived since is published.
        let later = NOW + HOUR_MS;
        let late_day = FakeGcp { descriptors: vec![daily], series: HashMap::from([(BYTES.to_string(), vec![bucket(vec![num(day - DAY_MS, 10.0), num(day, 20.0), num(day + MINUTE_MS, 5.0)])])]), ..Default::default() };
        let (crawl, sink) = run(&late_day, &json!({}), Some(later - LAG_MS - 10 * MINUTE_MS), &state, later).await;
        assert_eq!(values(&sink).iter().map(|p| (p.1, p.2)).collect::<Vec<_>>(), [(day + MINUTE_MS, 5.0)]);
        assert_eq!(crawl.state.unwrap().marks.values().copied().collect::<Vec<_>>(), [day + MINUTE_MS]);
    }

    #[tokio::test]
    async fn failures_publish_nothing_and_become_issues() {
        let denied = GcpError::Forbidden { permission: Some("monitoring.timeSeries.list".into()), message: "denied".into() };
        let gcp = FakeGcp { descriptors: vec![d(CPU, "GAUGE", "DOUBLE")], fail: Some(denied.clone()), ..Default::default() };
        let (crawl, sink) = run(&gcp, &json!({}), None, &State::default(), NOW).await;
        assert_eq!(crawl, Crawl { outcome: Outcome::Failed(issue(&denied)), state: None });
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(issue(&denied).missing_action.as_deref(), Some("monitoring.timeSeries.list"));
        assert_eq!(issue(&GcpError::Disabled("x".into())).backoff, Backoff::Day);
    }

    #[tokio::test]
    async fn throttled_reads_publish_the_rest_and_are_retried_on_the_next_runs() {
        let gcp = FakeGcp { descriptors: vec![d(CPU, "GAUGE", "DOUBLE"), d(SQL, "GAUGE", "DOUBLE")], series: HashMap::from([(CPU.to_string(), vec![instance("111", "prod")]), (SQL.to_string(), vec![Series { resource_type: "cloudsql_database".into(), points: minutes(0.2), ..Default::default() }])]), ..Default::default() };
        gcp.failing.lock().unwrap().insert(SQL.into(), GcpError::Throttled("Quota exceeded for quota metric 'Time series queries'".into()));
        let start = END - 600_000;
        let (crawl, sink) = run(&gcp, &json!({}), Some(start), &State::default(), NOW).await;
        let Outcome::Collected { end_ms, warning: Some(w), .. } = &crawl.outcome else { panic!("{:?}", crawl.outcome) };
        assert_eq!((*end_ms, w.kind), (END, "throttled"));
        assert!(w.message.contains("1 of 2 Cloud Monitoring reads failed and are retried on the next runs"), "{}", w.message);
        let names: HashSet<String> = published(&sink).into_iter().map(|p| p.0).collect();
        assert_eq!(names, HashSet::from(["gcp.gce.instance.cpu.utilization".to_string()]));
        let state = crawl.state.unwrap();
        assert_eq!(state.pending, [Pending { metric_type: SQL.into(), from: start, to: END, after: Some(start) }]);

        // The next run reads the failed window as well as its own.
        gcp.failing.lock().unwrap().clear();
        gcp.calls.lock().unwrap().clear();
        let later = NOW + 600_000;
        let (crawl, sink) = run(&gcp, &json!({}), Some(END), &state, later).await;
        assert!(matches!(crawl.outcome, Outcome::Collected { warning: None, .. }), "{:?}", crawl.outcome);
        assert!(crawl.state.unwrap().pending.is_empty());
        let mut sql_windows: Vec<(i64, i64)> = gcp.calls.lock().unwrap().iter().filter(|c| c.0 == SQL).map(|c| (c.1, c.2)).collect();
        sql_windows.sort();
        assert_eq!(sql_windows, [(start, END), (END, END + 600_000)]);
        assert_eq!(published(&sink).iter().filter(|p| p.0 == "gcp.cloudsql.database.cpu.utilization").count(), 10);

        // A pending read older than a day is dropped; one of a metric no longer collected too.
        let stale = State { pending: vec![Pending { metric_type: SQL.into(), from: END - 2 * DAY_MS, to: END - 2 * DAY_MS + 600_000, after: None }, Pending { metric_type: "gone.googleapis.com/x".into(), from: start, to: END, after: Some(start) }], ..Default::default() };
        gcp.calls.lock().unwrap().clear();
        run(&gcp, &json!({}), Some(END), &stale, later).await;
        assert_eq!(gcp.calls.lock().unwrap().len(), 2);

        // Every read throttled: the run fails and the watermark stays.
        gcp.failing.lock().unwrap().extend([(CPU.to_string(), GcpError::Throttled("busy".into())), (SQL.to_string(), GcpError::Throttled("busy".into()))]);
        let (crawl, sink) = run(&gcp, &json!({}), Some(END), &state, later).await;
        assert!(matches!(crawl.outcome, Outcome::Failed(Issue { kind: "throttled", .. })), "{:?}", crawl.outcome);
        assert_eq!(crawl.state, None);
        assert!(sink.records.lock().unwrap().is_empty());

        // A missing permission still fails the run, whatever else was read.
        gcp.failing.lock().unwrap().clear();
        let denied = GcpError::Forbidden { permission: Some("monitoring.timeSeries.list".into()), message: "denied".into() };
        gcp.failing.lock().unwrap().insert(SQL.into(), denied.clone());
        let (crawl, sink) = run(&gcp, &json!({}), Some(END), &State::default(), later).await;
        assert_eq!(crawl, Crawl { outcome: Outcome::Failed(issue(&denied)), state: None });
        assert!(sink.records.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn discovery_lists_the_projects_of_each_service_account_and_runs_keep_their_marks() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "gcp-crawl").await;
        let created = accounts::create_row(st, &org, json!({ "client_email": "sa@example-storefront.iam.gserviceaccount.com" }).as_object().unwrap()).await.unwrap();
        let crawler = GcpCrawler { api: Arc::new(FakeGcp::default()), sink: Arc::new(FakeSink::default()), interval_secs: 600 };
        assert!(crawler.desired(st).await.unwrap().iter().any(|d| d.0 == org && d.1 == created.id && d.2.is_empty()));
        let key = JobKey { provider: PROVIDER.into(), org_id: org.clone(), account_id: created.id.clone(), scope: "".into(), collected_until: None };
        assert_eq!(crawler.run(st, &key, NOW).await.unwrap(), Outcome::Discovered(vec!["example-storefront".into()]));
        let project = JobKey { scope: "example-storefront".into(), ..key };
        assert_eq!(load_state(st, &project).await.unwrap(), State::default());
        let state = State { marks: HashMap::from([("k".to_string(), 1)]), pending: vec![Pending { metric_type: CPU.into(), from: 1, to: 2, after: Some(1) }] };
        save_state(st, &project, &state).await.unwrap();
        assert_eq!(load_state(st, &project).await.unwrap(), state);
    }
}
