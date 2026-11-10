//! The authentik poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider `authentik`, one job per
//! connection, scope [`SCOPE`]), so several servers share the connections (`FOR UPDATE SKIP LOCKED`).
//!
//! authentik's events API has no time filter, but its pages oldest first (`ordering=created`) only grow at the end,
//! so a run ([`sync_events`], every [`INTERVAL_SECS`]) finds the page its window starts on ([`window_start`]: the
//! cursor; the first sync looks back [`FIRST_SYNC_LOOKBACK_MS`]) by a search from the last page ([`start_page`]: two
//! requests when it keeps up, a few more when it catches up), reads up to [`MAX_PAGES`] pages from there, sends the
//! events older than [`SETTLE_MS`] as logs, oldest first (JSON on `oc.logs.json`, with the org's headers), and moves
//! the cursor (`collected_until`) to the last one sent. A backlog larger than a run's pages is read on by the next
//! runs: no event of the window is skipped. Events at the cursor's instant are read again and stored once (a log's
//! identity comes from the event's id). A failed request publishes nothing, so the next run reads the same window again.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::api::{self, AuthentikApi, AuthentikError, PAGE_SIZE};
use super::logs::log_of;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::state::AppState;

pub const PROVIDER: &str = "authentik";
/// The Kafka topic the log ingester reads JSON logs from.
pub const LOGS_TOPIC: &str = "oc.logs.json";
/// A connection's only job scope.
pub const SCOPE: &str = "events";
/// How often a connection is polled.
pub const INTERVAL_SECS: i64 = 60;
/// How far back the first sync goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 86_400_000;
/// Events younger than this may still be committing (authentik stamps `created` before its row is visible): a run
/// leaves them to the next, so a run can start at its cursor.
pub const SETTLE_MS: i64 = 30_000;
/// Pages of events read per run (past the search for the first).
pub const MAX_PAGES: usize = 20;
/// Logs per Kafka record (well under the broker's 1 MiB message limit).
pub const LOGS_PER_RECORD: usize = 200;

/// An org's connection, with its token unsealed.
#[derive(Clone, Debug)]
pub struct Connection {
    pub base_url: String,
    pub token: String,
}

impl Connection {
    /// The instance's host name, the logs' `host`.
    pub fn host(&self) -> String {
        url::Url::parse(&self.base_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| self.base_url.clone())
    }
}

/// The oldest event time a run reads: the cursor, never further back than the first sync's lookback.
pub fn window_start(cursor: Option<i64>, now_ms: i64) -> i64 {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    cursor.map_or(floor, |c| c.max(floor))
}

/// An authentik error as the connection's issue.
pub fn issue(e: &AuthentikError) -> Issue {
    let (kind, backoff) = match e {
        AuthentikError::Throttled(_) => ("throttled", Backoff::Exponential),
        AuthentikError::Auth(_) => ("auth", Backoff::Interval),
        AuthentikError::NotFound(_) => ("access_denied", Backoff::Interval),
        AuthentikError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

fn created_ms(event: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(event["created"].as_str()?).ok().map(|t| t.timestamp_millis())
}

/// A page of events as read.
struct Page {
    events: Vec<Value>,
    total_pages: usize,
    next: bool,
}

/// The pages a run has read, so the search's pages aren't requested again.
struct Pages<'a> {
    api: &'a dyn AuthentikApi,
    conn: &'a Connection,
    read: HashMap<usize, Page>,
}

impl Pages<'_> {
    async fn load(&mut self, page: usize) -> Result<&Page, AuthentikError> {
        if !self.read.contains_key(&page) {
            let answer = self.api.get(&self.conn.base_url, &self.conn.token, &api::events_path(page)).await?;
            let Some(results) = answer["results"].as_array() else { return Err(AuthentikError::Other("authentik answered without a list of events".into())) };
            let total_pages = answer.pointer("/pagination/total_pages").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
            let next = answer.pointer("/pagination/next").and_then(Value::as_u64).unwrap_or(0) > 0;
            self.read.insert(page, Page { events: results.clone(), total_pages, next });
        }
        Ok(&self.read[&page])
    }

    /// Whether the page's first event is older than `start` (the window starts on it or on a later page).
    async fn starts_before(&mut self, page: usize, start: i64) -> Result<bool, AuthentikError> {
        Ok(self.load(page).await?.events.iter().find_map(created_ms).is_some_and(|at| at < start))
    }

    async fn take(&mut self, page: usize) -> Result<Page, AuthentikError> {
        self.load(page).await?;
        Ok(self.read.remove(&page).expect("loaded"))
    }
}

/// The page holding the window's oldest event: the last page starting before `start` (pages oldest first), found by
/// galloping back from the last page, then bisecting.
async fn start_page(pages: &mut Pages<'_>, start: i64) -> Result<usize, AuthentikError> {
    let total = pages.load(1).await?.total_pages;
    if !pages.starts_before(1, start).await? {
        return Ok(1);
    }
    if pages.starts_before(total, start).await? {
        return Ok(total);
    }
    // Page `lo` starts before the window, page `hi` doesn't.
    let (mut lo, mut hi, mut step) = (1, total, 1);
    while total - step > lo {
        let probe = total - step;
        if pages.starts_before(probe, start).await? {
            lo = probe;
            break;
        }
        hi = probe;
        step *= 2;
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if pages.starts_before(mid, start).await? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

/// The window's settled events, oldest first, and whether pages were left for the next run.
async fn read_window(pages: &mut Pages<'_>, start: i64, settled: i64) -> Result<(Vec<(i64, Value)>, bool), AuthentikError> {
    let mut page = start_page(pages, start).await?;
    let mut events = Vec::new();
    for read in 1..=MAX_PAGES {
        let Page { events: results, next, .. } = pages.take(page).await?;
        for event in results {
            let Some(at) = created_ms(&event) else { continue };
            if at > settled {
                return Ok((events, false));
            }
            if at >= start {
                events.push((at, event));
            }
        }
        if !next {
            return Ok((events, false));
        }
        if read == MAX_PAGES {
            return Ok((events, true));
        }
        page += 1;
    }
    Ok((events, false))
}

/// Polls one connection: see the module documentation. `cursor`: the job's `collected_until`.
pub async fn sync_events(api: &dyn AuthentikApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, cursor: Option<i64>, now_ms: i64) -> Outcome {
    let start = window_start(cursor, now_ms);
    let mut pages = Pages { api, conn, read: HashMap::new() };
    let (events, more) = match read_window(&mut pages, start, now_ms - SETTLE_MS).await {
        Ok(read) => read,
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    let host = conn.host();
    let mut end_ms = events.last().map_or(cursor.unwrap_or(start), |(at, _)| *at);
    let logs: Vec<Value> = events.iter().map(|(_, e)| log_of(e, &host)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        let body = serde_json::to_vec(chunk).unwrap_or_default();
        if let Err(e) = sink.publish(org, headers, body).await {
            return Outcome::Failed(issue(&AuthentikError::Other(format!("Kafka: {e}"))));
        }
    }
    // A run's pages all at the window's first instant: step past it, or every run would read the same pages.
    let mut warning = None;
    if more && end_ms <= start {
        end_ms = start + 1;
        warning = Some(Issue {
            kind: "other",
            message: format!("more than {} events at one instant: the others of that instant were skipped", MAX_PAGES * PAGE_SIZE),
            missing_action: None,
            backoff: Backoff::Exponential,
        });
    }
    Outcome::Collected { end_ms, metrics: logs.len(), points: logs.len(), warning }
}

/// The poller: authentik through `api`, logs to `sink`.
pub struct AuthentikCrawler {
    pub api: Arc<dyn AuthentikApi>,
    pub sink: Arc<dyn MetricSink>,
}

/// An org's connection with its token unsealed, and its id.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<(String, Connection)>, sqlx::Error> {
    let row: Option<(String, String, String)> = sqlx::query_as("SELECT id::text, base_url, token_enc FROM authentik_connections WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row.and_then(|(id, base_url, sealed)| {
        let token = st.secrets.decrypt(&sealed).ok()?;
        Some((id, Connection { base_url, token }))
    }))
}

#[async_trait::async_trait]
impl Crawler for AuthentikCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One events job per connection.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.org_id, c.id::text FROM authentik_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, SCOPE.to_string())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, conn)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if id != job.account_id || job.scope != SCOPE {
            return Ok(Outcome::Idle);
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        Ok(sync_events(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &conn, job.collected_until, now_ms).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentik::fixtures::{self, FakeAuthentik};
    use crate::cloud::testing::FakeSink;

    fn conn() -> Connection {
        Connection { base_url: "https://portal.example.com".into(), token: "ak-test-token".into() }
    }

    fn ms(t: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp_millis()
    }

    #[test]
    fn the_window_starts_at_the_cursor_and_is_bounded() {
        let now = ms("2026-11-06T12:00:00Z");
        assert_eq!(window_start(None, now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(window_start(Some(now - 300_000), now), now - 300_000);
        assert_eq!(window_start(Some(now - 30 * 86_400_000), now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(conn().host(), "portal.example.com");
    }

    fn headers() -> Vec<(String, String)> {
        vec![("x-oc-org".into(), "example-org".into())]
    }

    /// The published logs, in order.
    fn published(sink: &FakeSink) -> Vec<Value> {
        sink.records.lock().unwrap().iter().flat_map(|(_, _, doc)| doc.as_array().unwrap().clone()).collect()
    }

    fn ids(sink: &FakeSink) -> Vec<String> {
        published(sink).iter().map(|l| l["authentik"]["event_id"].as_str().unwrap().to_string()).collect()
    }

    /// `n` logins a second apart, from `from` on.
    fn logins(prefix: &str, from: i64, n: usize) -> Vec<Value> {
        (0..n).map(|i| fixtures::login(&format!("{prefix}{i:05}"), &chrono::DateTime::from_timestamp_millis(from + i as i64 * 1000).unwrap().to_rfc3339())).collect()
    }

    #[tokio::test]
    async fn the_first_sync_sends_a_days_settled_events_oldest_first() {
        let api = FakeAuthentik::default();
        let now = ms("2026-11-06T12:00:00Z");
        let mut log = logins("old", ms("2026-11-05T10:00:00Z"), 250);
        log.extend(logins("e", ms("2026-11-06T11:00:00Z"), 150));
        log.push(fixtures::event("newest", "login_failed", "2026-11-06T11:59:20.900Z", serde_json::json!({ "username": "admin" })));
        log.push(fixtures::login("unsettled", "2026-11-06T11:59:50Z"));
        api.log(&log);
        let sink = FakeSink::default();
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), None, now).await;
        let ids = ids(&sink);
        assert_eq!(ids.len(), 151);
        assert_eq!((ids[0].as_str(), ids.last().unwrap().as_str()), ("e00000", "newest"), "oldest first, without the day before or the unsettled");
        assert!(published(&sink).iter().all(|l| l["host"] == "portal.example.com"));
        assert_eq!(sink.records.lock().unwrap()[0].1, headers(), "with the org's headers");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:59:20.900Z"), metrics: 151, points: 151, warning: None });
    }

    #[tokio::test]
    async fn a_run_that_keeps_up_reads_the_first_and_last_pages() {
        let api = FakeAuthentik::default();
        let mut log = logins("old", ms("2026-11-01T00:00:00Z"), 950);
        log.extend(logins("new", ms("2026-11-06T11:00:10Z"), 3));
        api.log(&log);
        let cursor = ms("2026-11-06T11:00:10Z");
        let sink = FakeSink::default();
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), ms("2026-11-06T11:01:00Z")).await;
        assert_eq!(api.requested(), vec![api::events_path(1), api::events_path(10)]);
        assert_eq!(ids(&sink), ["new00000", "new00001", "new00002"], "the event at the cursor is read again");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:00:12Z"), metrics: 3, points: 3, warning: None });
    }

    #[tokio::test]
    async fn a_backlog_past_a_runs_pages_is_read_on_by_the_next_runs_without_gaps() {
        let api = FakeAuthentik::default();
        let mut log = logins("old", ms("2026-11-04T00:00:00Z"), 1234);
        let burst = logins("b", ms("2026-11-06T10:00:00Z"), MAX_PAGES * PAGE_SIZE + 750);
        log.extend(burst.iter().cloned());
        api.log(&log);
        let now = ms("2026-11-06T12:00:00Z");
        let sink = FakeSink::default();
        let mut cursor = None;
        for _ in 0..2 {
            match sync_events(&api, &sink, &headers(), "example-org", &conn(), cursor, now).await {
                Outcome::Collected { end_ms, warning: None, .. } => cursor = Some(end_ms),
                other => panic!("{other:?}"),
            }
        }
        let mut sent = ids(&sink);
        assert_eq!(sent.len(), burst.len() + 1, "the event at the first run's cursor is sent twice");
        sent.dedup();
        assert_eq!(sent, burst.iter().map(|e| e["pk"].as_str().unwrap()).collect::<Vec<_>>(), "every event of the window, in order");
        assert_eq!(cursor, Some(ms(burst.last().unwrap()["created"].as_str().unwrap())));
        assert!(sink.records.lock().unwrap().iter().all(|(_, _, doc)| doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
    }

    #[tokio::test]
    async fn nothing_new_keeps_the_cursor() {
        let api = FakeAuthentik::default();
        api.log(&[]);
        let sink = FakeSink::default();
        let cursor = ms("2026-11-06T11:00:00Z");
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), cursor + 60_000).await;
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(outcome, Outcome::Collected { end_ms: cursor, metrics: 0, points: 0, warning: None });
    }

    #[tokio::test]
    async fn a_refused_token_or_kafka_failure_publishes_nothing_and_keeps_the_window() {
        let refused = FakeAuthentik { refuse: true, ..Default::default() };
        let sink = FakeSink::default();
        match sync_events(&refused, &sink, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await {
            Outcome::Failed(issue) => assert_eq!((issue.kind, issue.backoff), ("auth", Backoff::Interval)),
            other => panic!("{other:?}"),
        }
        assert!(sink.records.lock().unwrap().is_empty());
        let api = FakeAuthentik::default();
        api.log(&[fixtures::login("a", "2026-11-06T11:59:00Z")]);
        let down = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(sync_events(&api, &down, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await, Outcome::Failed(_)));
    }

    #[tokio::test]
    async fn a_runs_pages_at_one_instant_step_past_it_and_warn() {
        let api = FakeAuthentik::default();
        let log: Vec<Value> = (0..MAX_PAGES * PAGE_SIZE + 10).map(|i| fixtures::login(&format!("s{i}"), "2026-11-06T11:00:00Z")).collect();
        api.log(&log);
        let cursor = ms("2026-11-06T11:00:00Z");
        match sync_events(&api, &FakeSink::default(), &headers(), "example-org", &conn(), Some(cursor), ms("2026-11-06T12:00:00Z")).await {
            Outcome::Collected { end_ms, metrics, warning: Some(w), .. } => {
                assert_eq!((end_ms, metrics), (cursor + 1, MAX_PAGES * PAGE_SIZE));
                assert!(w.message.contains("one instant"), "{}", w.message);
            }
            other => panic!("{other:?}"),
        }
    }
}
