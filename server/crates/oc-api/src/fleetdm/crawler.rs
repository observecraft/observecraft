//! The FleetDM poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider `fleetdm`), so several servers share
//! the connections (`FOR UPDATE SKIP LOCKED`). Each connection has two jobs, run every [`INTERVAL_SECS`]:
//!
//! - [`ACTIVITIES`] ([`sync_activities`]): a run reads the activities created since its window's start
//!   ([`window_start`]: the cursor; the first sync looks back [`FIRST_SYNC_LOOKBACK_MS`]) in Fleet's order of creation
//!   (by id, `start_created_at`), up to [`MAX_ACTIVITY_PAGES`] pages, sends those older than [`SETTLE_MS`] as logs
//!   (JSON on `oc.logs.json`, with the org's headers) and moves the cursor (`collected_until`) to the last one sent. A
//!   backlog larger than a run's pages is read on by the next runs: no activity of the window is skipped. Activities
//!   of the cursor's second are read again and stored once (a log's identity comes from the activity's id). A failed
//!   request publishes nothing, so the next run reads the same window again.
//! - [`HOSTS`] ([`sync_hosts`]): the host counts, the global policies, the most widespread vulnerabilities and every
//!   host (up to [`MAX_HOST_PAGES`] pages) as gauges (`fleetdm.*`, OTLP JSON on `oc.otlp-json.metrics`); see
//!   [`super::hosts`].

use std::sync::Arc;

use serde_json::Value;

use super::api::{self, FleetApi, FleetError, ACTIVITIES_PAGE_SIZE, HOSTS_PAGE_SIZE};
use super::hosts::{host_points, policy_points, summary_points, vulnerability_points};
use super::logs::log_of;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::cloud::naming::Point;
use crate::state::AppState;

pub const PROVIDER: &str = "fleetdm";
/// The Kafka topic the log ingester reads JSON logs from.
pub const LOGS_TOPIC: &str = "oc.logs.json";
/// The activities job.
pub const ACTIVITIES: &str = "activities";
/// The hosts and policies job.
pub const HOSTS: &str = "hosts";
/// How often a connection is polled.
pub const INTERVAL_SECS: i64 = 60;
/// How far back the first sync goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 86_400_000;
/// Activities younger than this may still be committing: a run leaves them to the next, so a run can start at its
/// cursor.
pub const SETTLE_MS: i64 = 30_000;
/// Pages of activities read per run.
pub const MAX_ACTIVITY_PAGES: usize = 20;
/// Pages of hosts read per run (10,000 hosts).
pub const MAX_HOST_PAGES: usize = 50;
/// Logs per Kafka record (well under the broker's 1 MiB message limit).
pub const LOGS_PER_RECORD: usize = 200;
/// Points per Kafka record.
pub const POINTS_PER_RECORD: usize = 2_000;

/// An org's connection, with its token unsealed.
#[derive(Clone, Debug)]
pub struct Connection {
    pub base_url: String,
    pub token: String,
}

impl Connection {
    /// The Fleet server's host name: the logs' `host` and the metrics' `fleet_instance`.
    pub fn host(&self) -> String {
        url::Url::parse(&self.base_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| self.base_url.clone())
    }
}

/// The oldest activity time a run reads: the cursor, never further back than the first sync's lookback.
pub fn window_start(cursor: Option<i64>, now_ms: i64) -> i64 {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    cursor.map_or(floor, |c| c.max(floor))
}

/// A Fleet error as the job's issue.
pub fn issue(e: &FleetError) -> Issue {
    let (kind, backoff) = match e {
        FleetError::Throttled(_) => ("throttled", Backoff::Exponential),
        FleetError::Auth(_) => ("auth", Backoff::Interval),
        FleetError::NotFound(_) => ("access_denied", Backoff::Interval),
        FleetError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

fn created_ms(activity: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(activity["created_at"].as_str()?).ok().map(|t| t.timestamp_millis())
}

fn warning(message: String) -> Option<Issue> {
    Some(Issue { kind: "other", message, missing_action: None, backoff: Backoff::Exponential })
}

/// Polls one connection's activities: see the module documentation. `cursor`: the job's `collected_until`.
pub async fn sync_activities(api: &dyn FleetApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, cursor: Option<i64>, now_ms: i64) -> Outcome {
    let start = window_start(cursor, now_ms);
    let settled = now_ms - SETTLE_MS;
    let mut activities: Vec<(i64, Value)> = Vec::new();
    let mut more = false;
    'pages: for page in 0..MAX_ACTIVITY_PAGES {
        let answer = match api.get(&conn.base_url, &conn.token, &api::activities_path(page, start)).await {
            Ok(a) => a,
            Err(e) => return Outcome::Failed(issue(&e)),
        };
        let Some(results) = answer["activities"].as_array() else {
            // Fleet answers `null` when there are none.
            if answer.get("activities").is_some_and(Value::is_null) {
                break;
            }
            return Outcome::Failed(issue(&FleetError::Other("Fleet answered without a list of activities".into())));
        };
        for activity in results {
            let Some(at) = created_ms(activity) else { continue };
            if at > settled {
                break 'pages;
            }
            if at >= start {
                activities.push((at, activity.clone()));
            }
        }
        if results.len() < ACTIVITIES_PAGE_SIZE || !answer.pointer("/meta/has_next_results").and_then(Value::as_bool).unwrap_or(false) {
            break;
        }
        more = page + 1 == MAX_ACTIVITY_PAGES;
    }
    let host = conn.host();
    let mut end_ms = activities.last().map_or(cursor.unwrap_or(start), |(at, _)| *at);
    let logs: Vec<Value> = activities.iter().map(|(_, a)| log_of(a, &host)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        let body = serde_json::to_vec(chunk).unwrap_or_default();
        if let Err(e) = sink.publish(org, headers, body).await {
            return Outcome::Failed(issue(&FleetError::Other(format!("Kafka: {e}"))));
        }
    }
    // A run's pages all in the window's first second: step past it, or every run would read the same pages.
    let mut warning = None;
    if more && end_ms < start + 1000 {
        end_ms = start - start.rem_euclid(1000) + 1000;
        warning = self::warning(format!("more than {} activities in one second: the others of that second were skipped", MAX_ACTIVITY_PAGES * ACTIVITIES_PAGE_SIZE));
    }
    Outcome::Collected { end_ms, metrics: logs.len(), points: logs.len(), warning }
}

/// Polls one connection's host counts, global policies and hosts into `fleetdm.*` gauges at the poll's minute.
/// `metrics` of the outcome: the hosts read.
pub async fn sync_hosts(api: &dyn FleetApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, now_ms: i64) -> Outcome {
    let minute = now_ms - now_ms.rem_euclid(60_000);
    let instance = conn.host();
    let get = |path: String| async move { api.get(&conn.base_url, &conn.token, &path).await };
    let summary = match get(api::HOST_SUMMARY.into()).await {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    let mut points: Vec<Point> = summary_points(&summary, &instance, minute);
    match get(api::GLOBAL_POLICIES.into()).await {
        Ok(p) => points.extend(policy_points(&p, &instance, minute)),
        Err(e) => return Outcome::Failed(issue(&e)),
    }
    // Vulnerabilities: a Fleet without them (an older version, or a role that can't list them) still reports the rest.
    let mut skipped = None;
    match get(api::vulnerabilities_path()).await {
        Ok(v) => points.extend(vulnerability_points(&v, &instance, minute)),
        Err(FleetError::NotFound(_) | FleetError::Auth(_)) => {}
        Err(e) => skipped = Some(format!("the vulnerabilities couldn't be read: {}", e.message())),
    }
    let mut hosts = 0;
    let mut truncated = false;
    for page in 0..MAX_HOST_PAGES {
        let answer = match get(api::hosts_path(page)).await {
            Ok(a) => a,
            Err(e) => return Outcome::Failed(issue(&e)),
        };
        let Some(list) = answer["hosts"].as_array() else {
            return Outcome::Failed(issue(&FleetError::Other("Fleet answered without a list of hosts".into())));
        };
        hosts += list.len();
        points.extend(host_points(list, &instance, minute));
        if list.len() < HOSTS_PAGE_SIZE {
            break;
        }
        truncated = page + 1 == MAX_HOST_PAGES;
    }
    for chunk in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, crate::cloud::naming::otlp_json(PROVIDER, chunk)).await {
            return Outcome::Failed(issue(&FleetError::Other(format!("Kafka: {e}"))));
        }
    }
    let warning = truncated.then(|| format!("more than {} hosts: the others' metrics were skipped (the host counts include them)", MAX_HOST_PAGES * HOSTS_PAGE_SIZE)).or(skipped).and_then(warning);
    Outcome::Collected { end_ms: minute, metrics: hosts, points: points.len(), warning }
}

/// The poller: Fleet through `api`, logs to `logs`, metrics to `metrics`.
pub struct FleetCrawler {
    pub api: Arc<dyn FleetApi>,
    pub logs: Arc<dyn MetricSink>,
    pub metrics: Arc<dyn MetricSink>,
}

/// An org's connection with its token unsealed, and its id.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<(String, Connection)>, sqlx::Error> {
    let row: Option<(String, String, String)> = sqlx::query_as("SELECT id::text, base_url, token_enc FROM fleetdm_connections WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row.and_then(|(id, base_url, sealed)| {
        let token = st.secrets.decrypt(&sealed).ok()?;
        Some((id, Connection { base_url, token }))
    }))
}

#[async_trait::async_trait]
impl Crawler for FleetCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// The activities and hosts jobs of every connection.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.org_id, c.id::text FROM fleetdm_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().flat_map(|(org, id)| [ACTIVITIES, HOSTS].map(|scope| (org.clone(), id.clone(), scope.to_string()))).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, conn)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if id != job.account_id {
            return Ok(Outcome::Idle);
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        Ok(match job.scope.as_str() {
            ACTIVITIES => sync_activities(self.api.as_ref(), self.logs.as_ref(), &headers, &job.org_id, &conn, job.collected_until, now_ms).await,
            HOSTS => sync_hosts(self.api.as_ref(), self.metrics.as_ref(), &headers, &job.org_id, &conn, now_ms).await,
            _ => Outcome::Idle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::FakeSink;
    use crate::fleetdm::fixtures::{self, FakeFleet};
    use serde_json::json;

    fn conn() -> Connection {
        Connection { base_url: "https://fleet.example.com".into(), token: "fleet-test-token".into() }
    }

    fn ms(t: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp_millis()
    }

    fn headers() -> Vec<(String, String)> {
        vec![("x-oc-org".into(), "example-org".into())]
    }

    /// The published logs, in order.
    fn published(sink: &FakeSink) -> Vec<Value> {
        sink.records.lock().unwrap().iter().flat_map(|(_, _, doc)| doc.as_array().unwrap().clone()).collect()
    }

    fn ids(sink: &FakeSink) -> Vec<String> {
        published(sink).iter().map(|l| l["fleetdm"]["activity_id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn the_window_starts_at_the_cursor_and_is_bounded() {
        let now = ms("2026-11-06T12:00:00Z");
        assert_eq!(window_start(None, now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(window_start(Some(now - 300_000), now), now - 300_000);
        assert_eq!(window_start(Some(now - 30 * 86_400_000), now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(conn().host(), "fleet.example.com");
    }

    /// `n` policies created a second apart from `from` on, with ids from `id`.
    fn created(id: u64, from: &str, n: usize) -> Vec<Value> {
        (0..n).map(|i| fixtures::created_policy(id + i as u64, &chrono::DateTime::from_timestamp_millis(ms(from) + i as i64 * 1000).unwrap().to_rfc3339())).collect()
    }

    #[tokio::test]
    async fn the_first_sync_sends_a_days_settled_activities_in_order() {
        let api = FakeFleet::default();
        let mut log = created(1, "2026-11-05T10:00:00Z", 30);
        log.extend(created(100, "2026-11-06T11:00:00Z", 150));
        log.push(fixtures::created_policy(900, "2026-11-06T11:59:50Z"));
        api.log(&log);
        let sink = FakeSink::default();
        let now = ms("2026-11-06T12:00:00Z");
        let outcome = sync_activities(&api, &sink, &headers(), "example-org", &conn(), None, now).await;
        assert_eq!(api.requested(), vec![api::activities_path(0, now - FIRST_SYNC_LOOKBACK_MS), api::activities_path(1, now - FIRST_SYNC_LOOKBACK_MS)], "from the day's start");
        let ids = ids(&sink);
        assert_eq!((ids.len(), ids[0].as_str(), ids.last().unwrap().as_str()), (150, "100", "249"), "the day's, oldest first, without the unsettled");
        assert!(published(&sink).iter().all(|l| l["host"] == "fleet.example.com"));
        assert_eq!(sink.records.lock().unwrap()[0].1, headers(), "with the org's headers");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:02:29Z"), metrics: 150, points: 150, warning: None });
    }

    #[tokio::test]
    async fn a_run_reads_from_its_cursor() {
        let api = FakeFleet::default();
        api.log(&[fixtures::created_policy(1, "2026-11-06T10:58:00Z"), fixtures::created_policy(2, "2026-11-06T11:00:00Z"), fixtures::created_policy(3, "2026-11-06T11:00:30Z")]);
        let sink = FakeSink::default();
        let cursor = ms("2026-11-06T11:00:00Z");
        let outcome = sync_activities(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), ms("2026-11-06T11:01:30Z")).await;
        assert_eq!(api.requested(), vec![api::activities_path(0, cursor)]);
        assert_eq!(ids(&sink), ["2", "3"], "the activity at the cursor is read again");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:00:30Z"), metrics: 2, points: 2, warning: None });
    }

    #[tokio::test]
    async fn a_backlog_past_a_runs_pages_is_read_on_by_the_next_runs_without_gaps() {
        let api = FakeFleet::default();
        let burst = created(1, "2026-11-06T10:00:00Z", MAX_ACTIVITY_PAGES * ACTIVITIES_PAGE_SIZE + 750);
        api.log(&burst);
        let now = ms("2026-11-06T12:00:00Z");
        let sink = FakeSink::default();
        let mut cursor = None;
        for _ in 0..2 {
            match sync_activities(&api, &sink, &headers(), "example-org", &conn(), cursor, now).await {
                Outcome::Collected { end_ms, warning: None, .. } => cursor = Some(end_ms),
                other => panic!("{other:?}"),
            }
        }
        let mut sent = ids(&sink);
        assert_eq!(sent.len(), burst.len() + 1, "the activity at the first run's cursor is sent twice");
        sent.dedup();
        assert_eq!(sent, (1..=burst.len()).map(|i| i.to_string()).collect::<Vec<_>>(), "every activity of the window, in order");
        assert_eq!(cursor, Some(ms(burst.last().unwrap()["created_at"].as_str().unwrap())));
        assert!(sink.records.lock().unwrap().iter().all(|(_, _, doc)| doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
    }

    #[tokio::test]
    async fn nothing_new_keeps_the_cursor() {
        let api = FakeFleet::default();
        api.log(&[]);
        let sink = FakeSink::default();
        let cursor = ms("2026-11-06T11:00:00Z");
        let outcome = sync_activities(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), cursor + 60_000).await;
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(outcome, Outcome::Collected { end_ms: cursor, metrics: 0, points: 0, warning: None });
    }

    #[tokio::test]
    async fn a_refused_token_or_kafka_failure_publishes_nothing() {
        let refused = FakeFleet { refuse: true, ..Default::default() };
        let sink = FakeSink::default();
        match sync_activities(&refused, &sink, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await {
            Outcome::Failed(issue) => assert_eq!((issue.kind, issue.backoff), ("auth", Backoff::Interval)),
            other => panic!("{other:?}"),
        }
        assert!(matches!(sync_hosts(&refused, &sink, &headers(), "example-org", &conn(), ms("2026-11-06T12:00:00Z")).await, Outcome::Failed(_)));
        assert!(sink.records.lock().unwrap().is_empty());
        let api = FakeFleet::default();
        api.log(&[fixtures::created_policy(1, "2026-11-06T11:59:00Z")]);
        let down = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(sync_activities(&api, &down, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await, Outcome::Failed(_)));
    }

    #[tokio::test]
    async fn a_runs_pages_in_one_second_step_past_it_and_warn() {
        let api = FakeFleet::default();
        api.log(&(1..=(MAX_ACTIVITY_PAGES * ACTIVITIES_PAGE_SIZE + 10) as u64).map(|i| fixtures::created_policy(i, "2026-11-06T11:00:00Z")).collect::<Vec<_>>());
        let cursor = ms("2026-11-06T11:00:00Z");
        match sync_activities(&api, &FakeSink::default(), &headers(), "example-org", &conn(), Some(cursor), ms("2026-11-06T12:00:00Z")).await {
            Outcome::Collected { end_ms, metrics, warning: Some(w), .. } => {
                assert_eq!((end_ms, metrics), (cursor + 1000, MAX_ACTIVITY_PAGES * ACTIVITIES_PAGE_SIZE));
                assert!(w.message.contains("one second"), "{}", w.message);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn the_hosts_job_sends_counts_policies_and_hosts_at_the_minute() {
        let api = FakeFleet::default();
        api.answer(api::HOST_SUMMARY, fixtures::host_summary());
        api.answer(api::GLOBAL_POLICIES, fixtures::policies());
        api.answer(api::hosts_path(0), json!({ "hosts": fixtures::hosts() }));
        api.answer(api::vulnerabilities_path(), fixtures::vulnerabilities());
        let sink = FakeSink::default();
        let now = ms("2026-11-06T12:00:42Z");
        let outcome = sync_hosts(&api, &sink, &headers(), "example-org", &conn(), now).await;
        assert_eq!(api.requested(), vec![api::HOST_SUMMARY.to_string(), api::GLOBAL_POLICIES.into(), api::vulnerabilities_path(), api::hosts_path(0)], "one page: fewer hosts than a page");
        let (summary, policies, hosts) = (summary_points(&fixtures::host_summary(), "fleet.example.com", 0), policy_points(&fixtures::policies(), "fleet.example.com", 0), host_points(&fixtures::hosts(), "fleet.example.com", 0));
        let expected = summary.len() + policies.len() + hosts.len() + vulnerability_points(&fixtures::vulnerabilities(), "fleet.example.com", 0).len();
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T12:00:00Z"), metrics: 3, points: expected, warning: None });
        assert_eq!(sink.records.lock().unwrap().len(), 1);

        // A Fleet that can't list vulnerabilities still reports the rest.
        api.answers.lock().unwrap().remove(&api::vulnerabilities_path());
        let outcome = sync_hosts(&api, &FakeSink::default(), &headers(), "example-org", &conn(), now).await;
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T12:00:00Z"), metrics: 3, points: summary.len() + policies.len() + hosts.len(), warning: None });
    }
}
