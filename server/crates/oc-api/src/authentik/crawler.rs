//! The authentik poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider `authentik`, one job per
//! connection, scope [`SCOPE`]), so several servers share the connections (`FOR UPDATE SKIP LOCKED`).
//!
//! authentik's events API has no time filter, so a run ([`sync_events`], every [`INTERVAL_SECS`]) reads pages
//! newest first until an event older than its window ([`window_start`]: the cursor minus [`CURSOR_OVERLAP_MS`];
//! the first sync looks back [`FIRST_SYNC_LOOKBACK_MS`]), then sends the window's events oldest first as logs
//! (JSON on `oc.logs.json`, with the org's headers) and moves the cursor (`collected_until`) to the newest. An
//! event read twice is stored once (its log's identity comes from the event's id). A failed request publishes
//! nothing, so the next run reads the same window again; past [`MAX_PAGES`] a run sends what it read and warns.

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
/// Each run reads again from a little before the cursor (events created in the same second).
pub const CURSOR_OVERLAP_MS: i64 = 60_000;
/// Pages of events read per run.
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

/// The oldest event time a run reads: a little before the cursor, never further back than the first sync's lookback.
pub fn window_start(cursor: Option<i64>, now_ms: i64) -> i64 {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    cursor.map_or(floor, |c| (c - CURSOR_OVERLAP_MS).max(floor))
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

/// Polls one connection: see the module documentation. `cursor`: the job's `collected_until`.
pub async fn sync_events(api: &dyn AuthentikApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, cursor: Option<i64>, now_ms: i64) -> Outcome {
    let start = window_start(cursor, now_ms);
    let mut events: Vec<(i64, Value)> = Vec::new();
    let mut truncated = false;
    'pages: for page in 1..=MAX_PAGES {
        let answer = match api.get(&conn.base_url, &conn.token, &api::events_path(page)).await {
            Ok(a) => a,
            Err(e) => return Outcome::Failed(issue(&e)),
        };
        let Some(results) = answer["results"].as_array() else {
            return Outcome::Failed(issue(&AuthentikError::Other("authentik answered without a list of events".into())));
        };
        for event in results {
            let Some(at) = created_ms(event) else { continue };
            if at < start {
                break 'pages;
            }
            events.push((at, event.clone()));
        }
        if results.len() < PAGE_SIZE || answer.pointer("/pagination/next").and_then(Value::as_u64).unwrap_or(0) == 0 {
            break;
        }
        truncated = page == MAX_PAGES;
    }
    let host = conn.host();
    let end_ms = events.iter().map(|(at, _)| *at).max().unwrap_or(cursor.unwrap_or(start));
    let logs: Vec<Value> = events.iter().rev().map(|(_, e)| log_of(e, &host)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        let body = serde_json::to_vec(chunk).unwrap_or_default();
        if let Err(e) = sink.publish(org, headers, body).await {
            return Outcome::Failed(issue(&AuthentikError::Other(format!("Kafka: {e}"))));
        }
    }
    let warning = truncated.then(|| Issue {
        kind: "other",
        message: format!("more than {} events in a minute: older events of this run were skipped", MAX_PAGES * PAGE_SIZE),
        missing_action: None,
        backoff: Backoff::Exponential,
    });
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
    fn the_window_overlaps_the_cursor_and_is_bounded() {
        let now = ms("2026-11-06T12:00:00Z");
        assert_eq!(window_start(None, now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(window_start(Some(now - 300_000), now), now - 300_000 - CURSOR_OVERLAP_MS);
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

    #[tokio::test]
    async fn the_first_sync_sends_a_days_events_oldest_first() {
        let api = FakeAuthentik::default();
        let now = ms("2026-11-06T12:00:00Z");
        let mut first: Vec<Value> = (0..PAGE_SIZE).map(|i| fixtures::login(&format!("e{i:03}"), &format!("2026-11-06T11:{:02}:{:02}Z", 59 - i / 60, 59 - i % 60))).collect();
        first[0] = fixtures::event("newest", "login_failed", "2026-11-06T11:59:59.900Z", serde_json::json!({ "username": "admin" }));
        api.answer(api::events_path(1), fixtures::page(first, 2));
        // The second page crosses the window: its older events stay out.
        api.answer(api::events_path(2), fixtures::page(vec![fixtures::login("in", "2026-11-05T12:00:01Z"), fixtures::login("out", "2026-11-05T11:59:59Z")], 0));
        let sink = FakeSink::default();
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), None, now).await;
        let logs = published(&sink);
        assert_eq!(logs.len(), PAGE_SIZE + 1);
        assert_eq!(logs[0]["authentik"]["event_id"], "in", "oldest first");
        assert_eq!(logs.last().unwrap()["authentik"]["event_id"], "newest");
        assert!(logs.iter().all(|l| l["host"] == "portal.example.com"));
        assert_eq!(sink.records.lock().unwrap()[0].1, headers(), "with the org's headers");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:59:59.900Z"), metrics: PAGE_SIZE + 1, points: PAGE_SIZE + 1, warning: None });
    }

    #[tokio::test]
    async fn a_run_reads_from_its_cursor_and_stops_at_the_first_older_page() {
        let api = FakeAuthentik::default();
        let cursor = ms("2026-11-06T11:00:00Z");
        api.answer(
            api::events_path(1),
            fixtures::page(vec![fixtures::login("new", "2026-11-06T11:00:30Z"), fixtures::login("overlap", "2026-11-06T10:59:30Z"), fixtures::login("old", "2026-11-06T10:58:00Z")], 2),
        );
        let sink = FakeSink::default();
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), ms("2026-11-06T11:01:00Z")).await;
        assert_eq!(api.requested(), vec![api::events_path(1)], "page 2 is older than the window");
        let ids: Vec<Value> = published(&sink).iter().map(|l| l["authentik"]["event_id"].clone()).collect();
        assert_eq!(ids, ["overlap", "new"], "a minute of overlap is read again");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:00:30Z"), metrics: 2, points: 2, warning: None });
    }

    #[tokio::test]
    async fn nothing_new_keeps_the_cursor() {
        let api = FakeAuthentik::default();
        api.answer(api::events_path(1), fixtures::page(vec![], 0));
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
        api.answer(api::events_path(1), fixtures::page(vec![fixtures::login("a", "2026-11-06T11:59:00Z")], 0));
        let down = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(sync_events(&api, &down, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await, Outcome::Failed(_)));
    }

    #[tokio::test]
    async fn past_the_page_budget_a_run_sends_what_it_read_and_warns() {
        let api = FakeAuthentik::default();
        for page in 1..=MAX_PAGES {
            let events = (0..PAGE_SIZE).map(|i| fixtures::login(&format!("p{page}-{i}"), "2026-11-06T11:59:00Z")).collect();
            api.answer(api::events_path(page), fixtures::page(events, page as u64 + 1));
        }
        let sink = FakeSink::default();
        match sync_events(&api, &sink, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await {
            Outcome::Collected { metrics, warning: Some(w), .. } => {
                assert_eq!(metrics, MAX_PAGES * PAGE_SIZE);
                assert!(w.message.contains("older events"), "{}", w.message);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(api.requested().len(), MAX_PAGES);
        assert!(sink.records.lock().unwrap().iter().all(|(_, _, doc)| doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
    }
}
