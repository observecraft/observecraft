//! The NetBird poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider `netbird`), so several servers
//! share the connections (`FOR UPDATE SKIP LOCKED`). Each connection has two jobs, run every [`INTERVAL_SECS`]:
//!
//! - [`EVENTS`] ([`sync_events`]): NetBird answers its newest audit events in one list, with no time filter; a run
//!   keeps those since its window ([`window_start`]: the cursor minus [`CURSOR_OVERLAP_MS`]; the first sync looks
//!   back [`FIRST_SYNC_LOOKBACK_MS`]) and sends them oldest first as logs (JSON on `oc.logs.json`, with the org's
//!   headers), then moves the cursor (`collected_until`) to the newest. An event read twice is stored once (its
//!   log's identity comes from the event's id). A failed request publishes nothing, so the next run reads the same
//!   window again.
//! - [`PEERS`] ([`sync_peers`]): the peers' state as gauges (`netbird.peer.*`, OTLP JSON on `oc.otlp-json.metrics`).

use std::sync::Arc;

use serde_json::Value;

use super::api::{self, NetbirdApi, NetbirdError};
use super::logs::log_of;
use super::peers::points_of;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::state::AppState;

pub const PROVIDER: &str = "netbird";
/// The Kafka topic the log ingester reads JSON logs from.
pub const LOGS_TOPIC: &str = "oc.logs.json";
/// The audit events job.
pub const EVENTS: &str = "events";
/// The peers job.
pub const PEERS: &str = "peers";
/// How often a connection is polled.
pub const INTERVAL_SECS: i64 = 60;
/// How far back the first sync goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 86_400_000;
/// Each run reads again from a little before the cursor (events recorded in the same second).
pub const CURSOR_OVERLAP_MS: i64 = 60_000;
/// The most events NetBird answers (`GetEvents`' limit): a full answer inside the window may have left some out.
pub const MAX_EVENTS: usize = 10_000;
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
    /// The management server's host name: the logs' `host` and the metrics' `netbird_instance`.
    pub fn host(&self) -> String {
        url::Url::parse(&self.base_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| self.base_url.clone())
    }
}

/// The oldest event time a run keeps: a little before the cursor, never further back than the first sync's lookback.
pub fn window_start(cursor: Option<i64>, now_ms: i64) -> i64 {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    cursor.map_or(floor, |c| (c - CURSOR_OVERLAP_MS).max(floor))
}

/// A NetBird error as the job's issue.
pub fn issue(e: &NetbirdError) -> Issue {
    let (kind, backoff) = match e {
        NetbirdError::Throttled(_) => ("throttled", Backoff::Exponential),
        NetbirdError::Auth(_) => ("auth", Backoff::Interval),
        NetbirdError::NotFound(_) => ("access_denied", Backoff::Interval),
        NetbirdError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

fn timestamp_ms(event: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(event["timestamp"].as_str()?).ok().map(|t| t.timestamp_millis())
}

async fn list(api: &dyn NetbirdApi, conn: &Connection, path: &str, what: &str) -> Result<Vec<Value>, Issue> {
    match api.get(&conn.base_url, &conn.token, path).await {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(issue(&NetbirdError::Other(format!("NetBird answered without a list of {what}")))),
        Err(e) => Err(issue(&e)),
    }
}

/// Polls one connection's audit events: see the module documentation. `cursor`: the job's `collected_until`.
pub async fn sync_events(api: &dyn NetbirdApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, cursor: Option<i64>, now_ms: i64) -> Outcome {
    let start = window_start(cursor, now_ms);
    let all = match list(api, conn, api::AUDIT_EVENTS, "events").await {
        Ok(a) => a,
        Err(issue) => return Outcome::Failed(issue),
    };
    let answered = all.len();
    let mut events: Vec<(i64, Value)> = all.into_iter().filter_map(|e| timestamp_ms(&e).map(|at| (at, e))).filter(|(at, _)| *at >= start).collect();
    events.sort_by_key(|(at, _)| *at);
    let host = conn.host();
    let end_ms = events.last().map(|(at, _)| *at).unwrap_or(cursor.unwrap_or(start));
    let logs: Vec<Value> = events.iter().map(|(_, e)| log_of(e, &host)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        let body = serde_json::to_vec(chunk).unwrap_or_default();
        if let Err(e) = sink.publish(org, headers, body).await {
            return Outcome::Failed(issue(&NetbirdError::Other(format!("Kafka: {e}"))));
        }
    }
    // NetBird answered its limit and every event is inside the window: older ones of the window weren't answered.
    let warning = (answered >= MAX_EVENTS && logs.len() == answered).then(|| Issue {
        kind: "other",
        message: format!("NetBird answered its newest {MAX_EVENTS} events, all in this window: older events of it were skipped"),
        missing_action: None,
        backoff: Backoff::Exponential,
    });
    Outcome::Collected { end_ms, metrics: logs.len(), points: logs.len(), warning }
}

/// Polls one connection's peers into `netbird.peer.*` gauges at the poll's minute.
pub async fn sync_peers(api: &dyn NetbirdApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, conn: &Connection, now_ms: i64) -> Outcome {
    let peers = match list(api, conn, api::PEERS, "peers").await {
        Ok(p) => p,
        Err(issue) => return Outcome::Failed(issue),
    };
    let minute = now_ms - now_ms.rem_euclid(60_000);
    let points = points_of(&peers, &conn.host(), minute);
    for chunk in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, crate::cloud::naming::otlp_json(PROVIDER, chunk)).await {
            return Outcome::Failed(issue(&NetbirdError::Other(format!("Kafka: {e}"))));
        }
    }
    Outcome::Collected { end_ms: minute, metrics: peers.len(), points: points.len(), warning: None }
}

/// The poller: NetBird through `api`, logs to `logs`, metrics to `metrics`.
pub struct NetbirdCrawler {
    pub api: Arc<dyn NetbirdApi>,
    pub logs: Arc<dyn MetricSink>,
    pub metrics: Arc<dyn MetricSink>,
}

/// An org's connection with its token unsealed, and its id.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<(String, Connection)>, sqlx::Error> {
    let row: Option<(String, String, String)> = sqlx::query_as("SELECT id::text, base_url, token_enc FROM netbird_connections WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row.and_then(|(id, base_url, sealed)| {
        let token = st.secrets.decrypt(&sealed).ok()?;
        Some((id, Connection { base_url, token }))
    }))
}

#[async_trait::async_trait]
impl Crawler for NetbirdCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// The events and peers jobs of every connection.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.org_id, c.id::text FROM netbird_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().flat_map(|(org, id)| [EVENTS, PEERS].map(|scope| (org.clone(), id.clone(), scope.to_string()))).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, conn)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if id != job.account_id {
            return Ok(Outcome::Idle);
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        Ok(match job.scope.as_str() {
            EVENTS => sync_events(self.api.as_ref(), self.logs.as_ref(), &headers, &job.org_id, &conn, job.collected_until, now_ms).await,
            PEERS => sync_peers(self.api.as_ref(), self.metrics.as_ref(), &headers, &job.org_id, &conn, now_ms).await,
            _ => Outcome::Idle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::{self, FakeSink};
    use crate::netbird::fixtures::{self, FakeNetbird};
    use serde_json::json;

    fn conn() -> Connection {
        Connection { base_url: "https://netbird.example.com".into(), token: "nbp_test_token".into() }
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

    #[test]
    fn the_window_overlaps_the_cursor_and_is_bounded() {
        let now = ms("2026-11-06T12:00:00Z");
        assert_eq!(window_start(None, now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(window_start(Some(now - 300_000), now), now - 300_000 - CURSOR_OVERLAP_MS);
        assert_eq!(window_start(Some(now - 30 * 86_400_000), now), now - FIRST_SYNC_LOOKBACK_MS);
        assert_eq!(conn().host(), "netbird.example.com");
    }

    #[tokio::test]
    async fn the_first_sync_sends_a_days_events_oldest_first() {
        let api = FakeNetbird::default();
        // Newest first, as NetBird answers; the last one is older than a day.
        api.answer(
            api::AUDIT_EVENTS,
            json!([fixtures::peer_added(3, "2026-11-06T11:59:00Z"), fixtures::peer_added(2, "2026-11-06T09:00:00Z"), fixtures::peer_added(1, "2026-11-05T11:00:00Z")]),
        );
        let sink = FakeSink::default();
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await;
        let ids: Vec<Value> = published(&sink).iter().map(|l| l["netbird"]["event_id"].clone()).collect();
        assert_eq!(ids, ["2", "3"], "oldest first, a day back");
        assert_eq!(api.requested(), [api::AUDIT_EVENTS], "one request: NetBird answers the whole list");
        assert!(published(&sink).iter().all(|l| l["host"] == "netbird.example.com"));
        assert_eq!(sink.records.lock().unwrap()[0].1, headers(), "with the org's headers");
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:59:00Z"), metrics: 2, points: 2, warning: None });
    }

    #[tokio::test]
    async fn a_run_reads_from_its_cursor_with_a_minute_of_overlap() {
        let api = FakeNetbird::default();
        api.answer(
            api::AUDIT_EVENTS,
            json!([fixtures::peer_added(3, "2026-11-06T11:00:30Z"), fixtures::peer_added(2, "2026-11-06T10:59:30Z"), fixtures::peer_added(1, "2026-11-06T10:58:00Z")]),
        );
        let sink = FakeSink::default();
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(ms("2026-11-06T11:00:00Z")), ms("2026-11-06T11:01:00Z")).await;
        let ids: Vec<Value> = published(&sink).iter().map(|l| l["netbird"]["event_id"].clone()).collect();
        assert_eq!(ids, ["2", "3"]);
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T11:00:30Z"), metrics: 2, points: 2, warning: None });
    }

    #[tokio::test]
    async fn nothing_new_keeps_the_cursor() {
        let api = FakeNetbird::default();
        api.answer(api::AUDIT_EVENTS, json!([]));
        let sink = FakeSink::default();
        let cursor = ms("2026-11-06T11:00:00Z");
        let outcome = sync_events(&api, &sink, &headers(), "example-org", &conn(), Some(cursor), cursor + 60_000).await;
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(outcome, Outcome::Collected { end_ms: cursor, metrics: 0, points: 0, warning: None });
    }

    #[tokio::test]
    async fn a_full_answer_inside_the_window_warns() {
        let api = FakeNetbird::default();
        let events: Vec<Value> = (0..MAX_EVENTS as u64).map(|i| fixtures::peer_added(i, "2026-11-06T11:59:00Z")).collect();
        api.answer(api::AUDIT_EVENTS, Value::Array(events));
        let sink = FakeSink::default();
        match sync_events(&api, &sink, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await {
            Outcome::Collected { metrics, warning: Some(w), .. } => {
                assert_eq!(metrics, MAX_EVENTS);
                assert!(w.message.contains("skipped"), "{}", w.message);
            }
            other => panic!("{other:?}"),
        }
        assert!(sink.records.lock().unwrap().iter().all(|(_, _, doc)| doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
    }

    #[tokio::test]
    async fn a_refused_token_a_wrong_answer_or_kafka_failure_publishes_nothing() {
        let refused = FakeNetbird { refuse: true, ..Default::default() };
        let sink = FakeSink::default();
        match sync_events(&refused, &sink, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await {
            Outcome::Failed(issue) => assert_eq!((issue.kind, issue.backoff), ("auth", Backoff::Interval)),
            other => panic!("{other:?}"),
        }
        assert!(matches!(sync_peers(&refused, &sink, &headers(), "example-org", &conn(), 0).await, Outcome::Failed(_)));
        let html = FakeNetbird::default();
        html.answer(api::AUDIT_EVENTS, json!({ "message": "not the API" }));
        assert!(matches!(sync_events(&html, &sink, &headers(), "example-org", &conn(), None, 0).await, Outcome::Failed(i) if i.message.contains("list of events")));
        assert!(sink.records.lock().unwrap().is_empty());
        let api = FakeNetbird::default();
        api.answer(api::AUDIT_EVENTS, json!([fixtures::peer_added(1, "2026-11-06T11:59:00Z")]));
        let down = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(sync_events(&api, &down, &headers(), "example-org", &conn(), None, ms("2026-11-06T12:00:00Z")).await, Outcome::Failed(_)));
    }

    #[tokio::test]
    async fn peers_become_gauges_at_the_polls_minute() {
        let api = FakeNetbird::default();
        api.answer(api::PEERS, Value::Array(fixtures::peers()));
        let sink = FakeSink::default();
        let outcome = sync_peers(&api, &sink, &headers(), "example-org", &conn(), ms("2026-11-06T12:00:42Z")).await;
        let points = testing::published(&sink);
        assert_eq!(points.len(), 3 * crate::netbird::peers::METRICS.len());
        assert!(points.iter().all(|(_, host, attrs)| host.is_none() && attrs.contains(&("netbird_instance".into(), "netbird.example.com".into()))));
        let doc = &sink.records.lock().unwrap()[0].2;
        assert_eq!(doc["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["gauge"]["dataPoints"][0]["timeUnixNano"], (ms("2026-11-06T12:00:00Z") as i128 * 1_000_000).to_string());
        assert_eq!(outcome, Outcome::Collected { end_ms: ms("2026-11-06T12:00:00Z"), metrics: 3, points: 21, warning: None });
    }
}
