//! The GitLab Audit Events poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider [`PROVIDER`], one job
//! per connection, scope [`SCOPE`]), so several servers share the connections (`FOR UPDATE SKIP LOCKED`).
//!
//! A run ([`sync_events`], every [`INTERVAL_SECS`]) reads the window from the cursor ([`window_start`]: the job's
//! `collected_until`; the first sync looks back [`FIRST_SYNC_LOOKBACK_MS`]) to [`SETTLE_MS`] ago. GitLab answers
//! audit events newest first, so a run walks the window back: it asks for the newest [`PER_PAGE`] events created
//! before its end, then for those created before the oldest one it got (that millisecond again, as GitLab's times
//! are finer than the answer's), until a page isn't full. Every event of the window is then sent as a log (JSON on
//! `oc.logs.json`, with the org's headers), oldest first, and the cursor moves to the window's end.
//!
//! A window larger than a run's [`MAX_PAGES`] is a backlog: the run sends what it read and leaves where it stopped
//! ([`Backlog`], on the connection); the next runs carry on walking back from there until they reach the cursor,
//! which then moves to the backlog's end. No event of the window is skipped. Events read twice (at a page's edge, a
//! window's end) are stored once: a log's identity comes from the event. A failed request publishes nothing, so the
//! next run reads the same pages again.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use super::api::{self, Scope, PER_PAGE};
use super::logs::log_of;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::gitlab::api::{GitlabApi, GitlabError};
use crate::state::AppState;

pub const PROVIDER: &str = "gitlab_audit";
/// The Kafka topic the log ingester reads JSON logs from.
pub const LOGS_TOPIC: &str = "oc.logs.json";
/// A connection's only job scope.
pub const SCOPE: &str = "events";
/// How often a connection is polled.
pub const INTERVAL_SECS: i64 = 60;
/// How far back the first sync goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 86_400_000;
/// Events younger than this may still be committing (some audit events are written by background jobs): a run
/// leaves them to the next.
pub const SETTLE_MS: i64 = 60_000;
/// Pages of events read per run.
pub const MAX_PAGES: usize = 20;
/// Logs per Kafka record (well under the broker's 1 MiB message limit).
pub const LOGS_PER_RECORD: usize = 200;

/// An org's connection, with its token unsealed.
#[derive(Clone, Debug)]
pub struct Connection {
    pub base_url: String,
    pub token: String,
    pub scope: Scope,
}

impl Connection {
    /// The instance's host name, the logs' `host`.
    pub fn host(&self) -> String {
        url::Url::parse(&self.base_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| self.base_url.clone())
    }
}

/// A window read over several runs: its end (where the cursor moves once it is read) and where the next run carries
/// on reading back from (epoch ms, both).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backlog {
    pub until_ms: i64,
    pub before_ms: i64,
}

/// A run's outcome and the backlog it leaves.
#[derive(Debug, PartialEq)]
pub struct Synced {
    pub outcome: Outcome,
    pub backlog: Option<Backlog>,
}

/// The oldest event time a run reads: the cursor, never further back than the first sync's lookback.
pub fn window_start(cursor: Option<i64>, now_ms: i64) -> i64 {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    cursor.map_or(floor, |c| c.max(floor))
}

/// A GitLab error as the connection's issue.
pub fn issue(e: &GitlabError) -> Issue {
    let (kind, backoff) = match e {
        GitlabError::Throttled(_) => ("throttled", Backoff::Exponential),
        GitlabError::Auth(_) => ("auth", Backoff::Interval),
        GitlabError::NotFound(_) => ("access_denied", Backoff::Interval),
        GitlabError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

fn created_ms(event: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(event["created_at"].as_str()?).ok().map(|t| t.timestamp_millis())
}

/// The events read walking back from `before` to `start`, and where the walk stopped when it ran out of pages
/// (`None`: it reached `start`). `crowded`: a full page within one millisecond, whose other events were skipped.
struct Walk {
    events: Vec<(i64, Value)>,
    stopped_at: Option<i64>,
    crowded: bool,
}

async fn walk_back(api: &dyn GitlabApi, conn: &Connection, start: i64, mut before: i64) -> Result<Walk, GitlabError> {
    let mut seen = HashSet::new();
    let mut walk = Walk { events: Vec::new(), stopped_at: None, crowded: false };
    for read in 1..=MAX_PAGES {
        let answer = api.get(&conn.base_url, &conn.token, &api::events_path(&conn.scope, start, before)).await?;
        let Some(page) = answer.as_array() else { return Err(GitlabError::Other("GitLab answered without a list of audit events".into())) };
        let mut oldest = before;
        for event in page {
            let Some(at) = created_ms(event) else { continue };
            oldest = oldest.min(at);
            let key = (event["entity_type"].as_str().unwrap_or_default().to_string(), event["id"].to_string());
            if (start..=before).contains(&at) && seen.insert(key) {
                walk.events.push((at, event.clone()));
            }
        }
        if page.len() < PER_PAGE {
            return Ok(walk);
        }
        // The next page: before the oldest event, its millisecond included.
        let mut next = oldest + 1;
        if next >= before {
            // A full page in one millisecond: step past it.
            next = oldest;
            walk.crowded = true;
        }
        if next < start {
            return Ok(walk);
        }
        before = next;
        if read == MAX_PAGES {
            walk.stopped_at = Some(before);
        }
    }
    Ok(walk)
}

/// Polls one connection: see the module documentation. `cursor`: the job's `collected_until`; `backlog`: the one the
/// previous run left.
#[allow(clippy::too_many_arguments)]
pub async fn sync_events(api: &dyn GitlabApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, cursor: Option<i64>, backlog: Option<Backlog>, now_ms: i64) -> Synced {
    let start = window_start(cursor, now_ms);
    let settled = now_ms - SETTLE_MS;
    let (until, before) = match backlog {
        Some(b) if b.before_ms > start && b.until_ms <= settled && b.before_ms <= b.until_ms => (b.until_ms, b.before_ms),
        _ => (settled, settled),
    };
    let kept = cursor.unwrap_or(start);
    if until <= start {
        return Synced { outcome: Outcome::Collected { end_ms: kept, metrics: 0, points: 0, warning: None }, backlog: None };
    }
    let walk = match walk_back(api, conn, start, before).await {
        Ok(w) => w,
        Err(e) => return Synced { outcome: Outcome::Failed(issue(&e)), backlog },
    };
    let mut events = walk.events;
    events.sort_by_key(|(at, e)| (*at, e["id"].as_i64()));
    let host = conn.host();
    let logs: Vec<Value> = events.iter().map(|(_, e)| log_of(e, &host)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        let body = serde_json::to_vec(chunk).unwrap_or_default();
        if let Err(e) = sink.publish(org, headers, body).await {
            return Synced { outcome: Outcome::Failed(issue(&GitlabError::Other(format!("Kafka: {e}")))), backlog };
        }
    }
    let warning = walk.crowded.then(|| Issue {
        kind: "other",
        message: format!("more than {PER_PAGE} audit events in one millisecond: the others of that millisecond were skipped"),
        missing_action: None,
        backoff: Backoff::Exponential,
    });
    let n = logs.len();
    match walk.stopped_at {
        // Read up to the cursor: the window is done.
        None => Synced { outcome: Outcome::Collected { end_ms: until, metrics: n, points: n, warning }, backlog: None },
        Some(at) => Synced { outcome: Outcome::Collected { end_ms: kept, metrics: n, points: n, warning }, backlog: Some(Backlog { until_ms: until, before_ms: at }) },
    }
}

/// The poller: GitLab through `api`, logs to `sink`.
pub struct GitlabAuditCrawler {
    pub api: Arc<dyn GitlabApi>,
    pub sink: Arc<dyn MetricSink>,
}

/// An org's connection with its token unsealed, its id and its backlog.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<(String, Connection, Option<Backlog>)>, sqlx::Error> {
    type Row = (String, String, String, String, String, Option<i64>, Option<i64>);
    let row: Option<Row> = sqlx::query_as("SELECT id::text, base_url, token_enc, scope, scope_path, backlog_until_ms, backlog_before_ms FROM gitlab_audit_connections WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.and_then(|(id, base_url, sealed, kind, path, until, before)| {
        let token = st.secrets.decrypt(&sealed).ok()?;
        let scope = Scope::parse(&kind, &path)?;
        let backlog = until.zip(before).map(|(until_ms, before_ms)| Backlog { until_ms, before_ms });
        Some((id, Connection { base_url, token, scope }, backlog))
    }))
}

#[async_trait::async_trait]
impl Crawler for GitlabAuditCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One events job per connection.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.org_id, c.id::text FROM gitlab_audit_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, SCOPE.to_string())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, conn, backlog)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if id != job.account_id || job.scope != SCOPE {
            return Ok(Outcome::Idle);
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        let synced = sync_events(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &conn, job.collected_until, backlog, now_ms).await;
        if synced.backlog != backlog {
            sqlx::query("UPDATE gitlab_audit_connections SET backlog_until_ms = $3, backlog_before_ms = $4 WHERE org_id = $1 AND id = $2::uuid")
                .bind(&job.org_id)
                .bind(&id)
                .bind(synced.backlog.map(|b| b.until_ms))
                .bind(synced.backlog.map(|b| b.before_ms))
                .execute(&st.postgres)
                .await?;
        }
        Ok(synced.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::FakeSink;
    use crate::gitlab_audit::fixtures::{self, FakeAudit};

    fn conn() -> Connection {
        Connection { base_url: "https://gitlab.example.com".into(), token: "glpat-audit-test".into(), scope: Scope::Instance }
    }

    fn ms(t: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp_millis()
    }

    /// A stored time: `ms` and `us` microseconds.
    fn at(ms: i64, us: i64) -> String {
        chrono::DateTime::from_timestamp_micros(ms * 1000 + us).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
    }

    fn headers() -> Vec<(String, String)> {
        vec![("x-oc-org".into(), "example-org".into())]
    }

    fn published(sink: &FakeSink) -> Vec<Value> {
        sink.records.lock().unwrap().iter().flat_map(|(_, _, doc)| doc.as_array().unwrap().clone()).collect()
    }

    fn ids(sink: &FakeSink) -> Vec<u64> {
        published(sink).iter().map(|l| l["gitlab"]["audit_event_id"].as_str().unwrap().parse().unwrap()).collect()
    }

    /// `n` events `step` ms apart from `from` on (a few microseconds into their millisecond), ids from `first`.
    fn changes(first: u64, from: i64, n: usize, step: i64) -> Vec<Value> {
        (0..n).map(|i| fixtures::changed(first + i as u64, &at(from + i as i64 * step, 1 + i as i64 % 999))).collect()
    }

    #[test]
    fn the_window_starts_at_the_cursor_and_is_bounded() {
        let now = ms("2026-11-06T12:00:00Z");
        assert_eq!(window_start(None, now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(window_start(Some(now - 300_000), now), now - 300_000);
        assert_eq!(window_start(Some(now - 30 * 86_400_000), now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(conn().host(), "gitlab.example.com");
    }

    #[tokio::test]
    async fn the_first_sync_sends_a_days_settled_events_oldest_first() {
        let now = ms("2026-11-06T12:00:00Z");
        let mut log = changes(1, ms("2026-11-05T10:00:00Z"), 30, 1000);
        log.extend(changes(100, ms("2026-11-06T09:00:00Z"), 250, 7_000));
        log.push(fixtures::failed_login(900, "2026-11-06T11:58:30.500Z", "root"));
        log.push(fixtures::changed(901, "2026-11-06T11:59:30Z"));
        let api = FakeAudit::with(log);
        let sink = FakeSink::default();
        let synced = sync_events(&api, &sink, &headers(), "example-org", &conn(), None, None, now).await;
        let ids = ids(&sink);
        assert_eq!(ids.len(), 251, "the day's settled events, without the day before or the unsettled");
        assert_eq!((ids[0], *ids.last().unwrap()), (100, 900), "oldest first");
        assert_eq!(api.requested().len(), 3, "pages of 100, 100 and 51");
        assert!(published(&sink).iter().all(|l| l["host"] == "gitlab.example.com" && l["ddsource"] == "gitlab-audit-events"));
        assert_eq!(sink.records.lock().unwrap()[0].1, headers(), "with the org's headers");
        assert_eq!(synced, Synced { outcome: Outcome::Collected { end_ms: now - SETTLE_MS, metrics: 251, points: 251, warning: None }, backlog: None });
    }

    #[tokio::test]
    async fn a_run_that_keeps_up_reads_one_page_from_its_cursor() {
        let cursor = ms("2026-11-06T11:00:00Z");
        let mut log = changes(1, cursor - 600_000, 500, 1000);
        log.extend(changes(1000, cursor, 3, 5_000));
        let api = FakeAudit::with(log);
        let sink = FakeSink::default();
        let now = cursor + 120_000;
        let synced = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), None, now).await;
        assert_eq!(api.requested(), vec![api::events_path(&Scope::Instance, cursor, now - SETTLE_MS)]);
        assert_eq!(ids(&sink), [1000, 1001, 1002], "the event at the cursor is read again");
        assert_eq!(synced.outcome, Outcome::Collected { end_ms: now - SETTLE_MS, metrics: 3, points: 3, warning: None });
    }

    #[tokio::test]
    async fn pages_cut_through_a_millisecond_lose_nothing() {
        // 150 events in two milliseconds: the second page asks for the first page's oldest millisecond again, whose
        // events GitLab stores later in that millisecond than the answer's time says.
        let t = ms("2026-11-06T11:30:00Z");
        let mut log = changes(1, t, 60, 0);
        log.extend(changes(500, t + 1, 90, 0));
        let api = FakeAudit::with(log);
        let sink = FakeSink::default();
        let synced = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(t - 60_000), None, t + 120_000).await;
        let mut sent = ids(&sink);
        sent.sort();
        sent.dedup();
        assert_eq!(sent.len(), 150);
        assert_eq!(ids(&sink).len(), 150, "the millisecond read twice is sent once");
        assert!(matches!(synced.outcome, Outcome::Collected { warning: None, .. }));
    }

    #[tokio::test]
    async fn a_backlog_past_a_runs_pages_is_read_on_by_the_next_runs_without_gaps() {
        let now = ms("2026-11-06T12:00:00Z");
        let burst = changes(1, ms("2026-11-06T08:00:00Z"), MAX_PAGES * PER_PAGE * 2 + 350, 1_500);
        let api = FakeAudit::with(burst.clone());
        let sink = FakeSink::default();
        let (mut cursor, mut backlog) = (None, None);
        let mut runs = 0;
        loop {
            runs += 1;
            let synced = sync_events(&api, &sink, &headers(), "example-org", &conn(), cursor, backlog, now).await;
            let Outcome::Collected { end_ms, warning: None, .. } = synced.outcome else { panic!("{:?}", synced.outcome) };
            cursor = Some(end_ms);
            backlog = synced.backlog;
            if backlog.is_none() {
                break;
            }
            assert_eq!(cursor, Some(now - FIRST_SYNC_LOOKBACK_MS), "the cursor waits for the backlog");
        }
        assert_eq!(runs, 3);
        assert_eq!(cursor, Some(now - SETTLE_MS), "then moves to the backlog's end");
        let mut sent = ids(&sink);
        sent.sort();
        sent.dedup();
        assert_eq!(sent, (1..=burst.len() as u64).collect::<Vec<_>>(), "every event of the window");
        assert!(sink.records.lock().unwrap().iter().all(|(_, _, doc)| doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
        // The next run reads from the cursor again.
        let api_requests = api.requested().len();
        let after = sync_events(&api, &FakeSink::default(), &headers(), "example-org", &conn(), cursor, None, now + 60_000).await;
        assert_eq!((api.requested().len() - api_requests, after.backlog), (1, None));
    }

    #[tokio::test]
    async fn a_full_page_in_one_millisecond_steps_past_it_and_warns() {
        let t = ms("2026-11-06T11:30:00Z");
        let api = FakeAudit::with(changes(1, t, PER_PAGE + 20, 0));
        let synced = sync_events(&api, &FakeSink::default(), &headers(), "example-org", &conn(), Some(t - 60_000), None, t + 120_000).await;
        match synced.outcome {
            Outcome::Collected { metrics, warning: Some(w), .. } => {
                assert_eq!(metrics, PER_PAGE);
                assert!(w.message.contains("one millisecond"), "{}", w.message);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn nothing_new_keeps_the_cursor_moving_to_the_settled_time() {
        let api = FakeAudit::with(vec![]);
        let sink = FakeSink::default();
        let cursor = ms("2026-11-06T11:00:00Z");
        let synced = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), None, cursor + 30_000).await;
        assert!(sink.records.lock().unwrap().is_empty() && api.requested().is_empty(), "nothing settled since the cursor");
        assert_eq!(synced.outcome, Outcome::Collected { end_ms: cursor, metrics: 0, points: 0, warning: None });
    }

    #[tokio::test]
    async fn a_refused_token_or_kafka_failure_publishes_nothing_and_keeps_the_window() {
        let refused = FakeAudit { refuse: true, ..Default::default() };
        let sink = FakeSink::default();
        let backlog = Some(Backlog { until_ms: ms("2026-11-06T11:00:00Z"), before_ms: ms("2026-11-06T10:00:00Z") });
        let synced = sync_events(&refused, &sink, &headers(), "example-org", &conn(), None, backlog, ms("2026-11-06T12:00:00Z")).await;
        match synced.outcome {
            Outcome::Failed(issue) => assert_eq!((issue.kind, issue.backoff), ("auth", Backoff::Interval)),
            other => panic!("{other:?}"),
        }
        assert_eq!(synced.backlog, backlog, "the backlog is kept");
        assert!(sink.records.lock().unwrap().is_empty());
        let api = FakeAudit::with(vec![fixtures::changed(1, "2026-11-06T11:58:00Z")]);
        let down = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(sync_events(&api, &down, &headers(), "example-org", &conn(), None, None, ms("2026-11-06T12:00:00Z")).await.outcome, Outcome::Failed(_)));
    }

    #[tokio::test]
    async fn a_group_connection_reads_the_groups_events() {
        let api = FakeAudit::with(vec![fixtures::changed(1, "2026-11-06T11:58:00Z")]);
        let group = Connection { scope: Scope::Group("example-org".into()), ..conn() };
        let sink = FakeSink::default();
        sync_events(&api, &sink, &headers(), "example-org", &group, None, None, ms("2026-11-06T12:00:00Z")).await;
        assert!(api.requested()[0].starts_with("/groups/example-org/audit_events?"));
        assert_eq!(ids(&sink), [1]);
    }
}
