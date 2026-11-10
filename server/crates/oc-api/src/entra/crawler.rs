//! The Entra ID logs poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider [`PROVIDER`]): one job per
//! app registration with Entra ID logs on and stream (scope `signins`, `audits`, `risk`), so several servers share
//! them (`FOR UPDATE SKIP LOCKED`).
//!
//! Graph lists sign-ins newest first and has no ordering to read them oldest first, so a run ([`sync`], every
//! [`INTERVAL_SECS`]) reads a closed window by its time filter, every page of it (`@odata.nextLink`), and only then
//! moves the cursor (`collected_until`) to the window's end: nothing in it is skipped. Graph makes events visible
//! minutes after their time, so each run also reads the [`OVERLAP_MS`] before the cursor again (as far as a run's
//! pages go, newest first); the ids sent from that overlap are kept ([`Seen`], Postgres `entra_log_state`), so an event is sent once (and a log's identity comes from the
//! event's id, so one sent again rewrites its row). A window spans at most [`MAX_WINDOW_MS`] past the cursor (the
//! first sync starts [`FIRST_SYNC_LOOKBACK_MS`] back); one whose new part holds more than [`MAX_PAGES`] pages is halved
//! until it fits. A failed request publishes nothing, so the next run reads the same window again.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::api::{list_path, GraphApi, Stream};
use super::logs::{event_ms, log_of};
use crate::azure::api::{AzureError, Credentials};
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::routes::datadog_api::azure_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "entra";
/// The Kafka topic the log ingester reads JSON logs from.
pub const LOGS_TOPIC: &str = "oc.logs.json";
/// How often a stream is polled.
pub const INTERVAL_SECS: i64 = 180;
/// How far back the first sync goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 86_400_000;
/// How far before the cursor a window starts, for events Graph shows late.
pub const OVERLAP_MS: i64 = 15 * 60_000;
/// A window ends this long before now.
pub const SETTLE_MS: i64 = 60_000;
/// The most a window reaches past the cursor.
pub const MAX_WINDOW_MS: i64 = 6 * 3_600_000;
/// A window holding more pages is halved; one this short is read up to its pages, with a warning.
pub const MIN_WINDOW_MS: i64 = 60_000;
/// Pages of a window read per run.
pub const MAX_PAGES: usize = 20;
/// Logs per Kafka record (well under the broker's 1 MiB message limit).
pub const LOGS_PER_RECORD: usize = 200;
/// The most ids kept from the overlap (the newest; older ones sent again rewrite their rows).
pub const MAX_SEEN: usize = 100_000;

/// The ids sent from the overlap: event id → event time (epoch ms).
pub type Seen = HashMap<String, i64>;

/// What a run did, and the ids to keep when it collected.
#[derive(Debug, PartialEq)]
pub struct Run {
    pub outcome: Outcome,
    pub seen: Option<Seen>,
}

/// A run's window: the overlap starts at `lo`, the new part at `from` (the cursor), both end at `hi`.
pub fn window(cursor: Option<i64>, now_ms: i64) -> (i64, i64, i64) {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    let from = cursor.map_or(floor, |c| c.max(floor));
    let lo = if cursor.is_some() { from - OVERLAP_MS } else { from };
    (from, lo, (from + MAX_WINDOW_MS).min(now_ms - SETTLE_MS))
}

/// A Graph error as the stream's issue: a missing license or permission names it.
pub fn issue(stream: Stream, e: &AzureError) -> Issue {
    let (kind, message, missing_action, backoff) = match e {
        AzureError::Auth(m) => ("auth", m.clone(), None, Backoff::Interval),
        AzureError::Forbidden { message, .. } => {
            let lower = message.to_ascii_lowercase();
            match stream.license() {
                Some(license) if lower.contains("premium") || lower.contains("licens") || lower.contains("sku") => {
                    ("access_denied", format!("Reading {} needs {license} on the tenant. Graph: {message}", label(stream)), None, Backoff::Interval)
                }
                _ => ("access_denied", format!("Grant the app registration Microsoft Graph's {} application permission (with admin consent). Graph: {message}", stream.permission()), Some(stream.permission().to_string()), Backoff::Interval),
            }
        }
        AzureError::Throttled(m) => ("throttled", m.clone(), None, Backoff::Exponential),
        AzureError::Rejected(m) | AzureError::Other(m) => ("other", m.clone(), None, Backoff::Exponential),
    };
    Issue { kind, message, missing_action, backoff }
}

/// The stream, as the issues name it.
pub fn label(stream: Stream) -> &'static str {
    match stream {
        Stream::SignIns => "sign-in logs",
        Stream::Audits => "audit logs",
        Stream::Risk => "risk detections",
    }
}

/// The events of `[lo, hi]`, up to [`MAX_PAGES`] pages, and whether that was all of them.
async fn read(api: &dyn GraphApi, creds: &Credentials, stream: Stream, lo: i64, hi: i64) -> Result<(Vec<Value>, bool), AzureError> {
    let mut events = Vec::new();
    let mut next = Some(list_path(stream, lo, hi));
    for _ in 0..MAX_PAGES {
        let Some(target) = next.take() else { break };
        let page = api.get(creds, &target).await?;
        let Some(values) = page["value"].as_array() else { return Err(AzureError::Other("Graph answered without a list of events".into())) };
        events.extend(values.iter().cloned());
        next = page["@odata.nextLink"].as_str().map(str::to_string);
    }
    Ok((events, next.is_none()))
}

/// Polls one stream: see the module documentation. `cursor`: the job's `collected_until`.
#[allow(clippy::too_many_arguments)]
pub async fn sync(api: &dyn GraphApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, creds: &Credentials, stream: Stream, cursor: Option<i64>, seen: Seen, now_ms: i64) -> Run {
    let (from, lo, mut hi) = window(cursor, now_ms);
    if hi <= from {
        return Run { outcome: Outcome::Collected { end_ms: cursor.unwrap_or(from), metrics: 0, points: 0, warning: None }, seen: Some(seen) };
    }
    let mut warning = None;
    // The overlap, newest first (late events are mostly near the cursor), as far as a run's pages go.
    let mut events = match lo < from {
        true => match read(api, creds, stream, lo, from - 1).await {
            Ok((events, _)) => events,
            Err(e) => return Run { outcome: Outcome::Failed(issue(stream, &e)), seen: None },
        },
        false => Vec::new(),
    };
    // The new part, whole.
    events.extend(loop {
        match read(api, creds, stream, from, hi).await {
            Err(e) => return Run { outcome: Outcome::Failed(issue(stream, &e)), seen: None },
            Ok((events, true)) => break events,
            Ok(_) if hi - from > MIN_WINDOW_MS => hi = from + (hi - from) / 2,
            // The pages read are the window's newest (Graph lists newest first): the others of it are skipped.
            Ok((events, false)) => {
                warning = Some(Issue {
                    kind: "other",
                    message: format!("more {} in a minute than a run reads ({} pages): some were skipped", label(stream), MAX_PAGES),
                    missing_action: None,
                    backoff: Backoff::Exponential,
                });
                break events;
            }
        }
    });
    let mut new: Vec<(i64, Value)> = events.into_iter().filter_map(|e| Some((event_ms(stream, &e)?, e))).filter(|(_, e)| e["id"].as_str().is_some_and(|id| !seen.contains_key(id))).collect();
    new.sort_by_key(|(at, _)| *at);
    new.dedup_by(|a, b| a.1["id"] == b.1["id"]);
    let logs: Vec<Value> = new.iter().map(|(_, e)| log_of(stream, e, &creds.tenant_id)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, serde_json::to_vec(chunk).unwrap_or_default()).await {
            return Run { outcome: Outcome::Failed(issue(stream, &AzureError::Other(format!("Kafka: {e}")))), seen: None };
        }
    }
    // The next window starts an overlap before `hi`: only the ids from there on are needed.
    let mut seen: Vec<(String, i64)> = seen.into_iter().chain(new.iter().filter_map(|(at, e)| Some((e["id"].as_str()?.to_string(), *at)))).filter(|(_, at)| *at >= hi - OVERLAP_MS).collect();
    seen.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    seen.truncate(MAX_SEEN);
    Run { outcome: Outcome::Collected { end_ms: hi, metrics: logs.len(), points: logs.len(), warning }, seen: Some(seen.into_iter().collect()) }
}

/// An app registration's Entra ID logs settings: which streams are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct Settings {
    pub sign_ins: bool,
    pub audits: bool,
    pub risk: bool,
}

impl Settings {
    pub fn streams(self) -> Vec<Stream> {
        [(Stream::SignIns, self.sign_ins), (Stream::Audits, self.audits), (Stream::Risk, self.risk)].into_iter().filter(|(_, on)| *on).map(|(s, _)| s).collect()
    }
}

/// The settings of an app registration of the org (None: Entra ID logs are off).
pub async fn settings(st: &AppState, org: &str, account: &str) -> Result<Option<Settings>, sqlx::Error> {
    sqlx::query_as("SELECT sign_ins, audits, risk FROM entra_log_settings WHERE org_id = $1 AND account_id::text = $2").bind(org).bind(account).fetch_optional(&st.postgres).await
}

/// The app registration's credentials (None: it's gone or has no client secret).
pub async fn credentials(st: &AppState, org: &str, account: &str) -> Result<Option<Credentials>, sqlx::Error> {
    let Some(row) = accounts::get_row(st, org, account).await? else { return Ok(None) };
    Ok(accounts::client_secret(st, org, account).await?.map(|client_secret| Credentials { tenant_id: row.tenant_name, client_id: row.client_id, client_secret }))
}

async fn load_seen(st: &AppState, key: &JobKey) -> Result<Seen, sqlx::Error> {
    let row: Option<Value> = sqlx::query_scalar("SELECT seen FROM entra_log_state WHERE org_id = $1 AND account_id = $2::uuid AND stream = $3").bind(&key.org_id).bind(&key.account_id).bind(&key.scope).fetch_optional(&st.postgres).await?;
    Ok(row.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

async fn save_seen(st: &AppState, key: &JobKey, seen: &Seen) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO entra_log_state (org_id, account_id, stream, seen) VALUES ($1, $2::uuid, $3, $4) ON CONFLICT (org_id, account_id, stream) DO UPDATE SET seen = EXCLUDED.seen, updated_at = now()")
        .bind(&key.org_id)
        .bind(&key.account_id)
        .bind(&key.scope)
        .bind(serde_json::to_value(seen).unwrap_or_default())
        .execute(&st.postgres)
        .await?;
    Ok(())
}

/// The poller: Graph through `api`, logs to `sink`.
pub struct EntraCrawler {
    pub api: Arc<dyn GraphApi>,
    pub sink: Arc<dyn MetricSink>,
}

#[async_trait::async_trait]
impl Crawler for EntraCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One job per app registration with Entra ID logs on and stream on.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String, bool, bool, bool)> =
            sqlx::query_as("SELECT s.org_id, s.account_id::text, s.sign_ins, s.audits, s.risk FROM entra_log_settings s JOIN orgs o ON o.id = s.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().flat_map(|(org, id, sign_ins, audits, risk)| Settings { sign_ins, audits, risk }.streams().into_iter().map(move |s| (org.clone(), id.clone(), s.scope().to_string()))).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(stream) = Stream::of_scope(&job.scope) else { return Ok(Outcome::Idle) };
        let Some(on) = settings(st, &job.org_id, &job.account_id).await? else { return Ok(Outcome::Idle) };
        if !on.streams().contains(&stream) {
            return Ok(Outcome::Idle);
        }
        let Some(creds) = credentials(st, &job.org_id, &job.account_id).await? else {
            let message = "This app registration has no client secret: ObserveCraft reads Microsoft Graph with a client secret.";
            return Ok(Outcome::Failed(Issue { kind: "auth", message: message.into(), missing_action: None, backoff: Backoff::Interval }));
        };
        let headers = jobs::org_headers(st, &job.org_id).await?;
        let seen = load_seen(st, job).await?;
        let run = sync(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &creds, stream, job.collected_until, seen, now_ms).await;
        if let (Outcome::Collected { .. }, Some(seen)) = (&run.outcome, &run.seen) {
            save_seen(st, job, seen).await?;
        }
        Ok(run.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::FakeSink;
    use crate::entra::fixtures::{self, FakeGraph, TENANT};

    fn creds() -> Credentials {
        Credentials { tenant_id: TENANT.into(), client_id: "c".into(), client_secret: "s".into() }
    }

    fn ms(t: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp_millis()
    }

    fn at(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    fn headers() -> Vec<(String, String)> {
        vec![("x-oc-org".into(), "example-org".into())]
    }

    fn sent(sink: &FakeSink) -> Vec<String> {
        sink.records.lock().unwrap().iter().flat_map(|(_, _, doc)| doc.as_array().unwrap().clone()).map(|l| l["properties"]["id"].as_str().unwrap().to_string()).collect()
    }

    /// `n` sign-ins a second apart from `from` on.
    fn sign_ins(prefix: &str, from: i64, n: usize) -> Vec<serde_json::Value> {
        (0..n).map(|i| fixtures::sign_in(&format!("{prefix}{i:05}"), &at(from + i as i64 * 1000), "jordan.patel@example.com", 0)).collect()
    }

    #[test]
    fn windows_start_an_overlap_before_the_cursor_and_are_bounded() {
        let now = ms("2026-11-06T12:00:00Z");
        assert_eq!(window(None, now), (now - FIRST_SYNC_LOOKBACK_MS, now - FIRST_SYNC_LOOKBACK_MS, now - FIRST_SYNC_LOOKBACK_MS + MAX_WINDOW_MS));
        let cursor = now - 300_000;
        assert_eq!(window(Some(cursor), now), (cursor, cursor - OVERLAP_MS, now - SETTLE_MS));
        assert_eq!(window(Some(now - 30 * 86_400_000), now).0, now - FIRST_SYNC_LOOKBACK_MS);
    }

    #[tokio::test]
    async fn a_run_sends_its_windows_events_oldest_first_and_moves_the_cursor_to_its_end() {
        let api = FakeGraph { page: 100, ..Default::default() };
        let now = ms("2026-11-06T12:00:00Z");
        let cursor = ms("2026-11-06T11:50:00Z");
        let mut events = sign_ins("old", cursor - 3_600_000, 5);
        events.extend(sign_ins("s", cursor - 60_000, 250));
        events.push(fixtures::sign_in("unsettled", &at(now - 10_000), "jordan.patel@example.com", 0));
        api.set("auditLogs/signIns", events);
        let sink = FakeSink::default();
        let run = sync(&api, &sink, &headers(), "example-org", &creds(), Stream::SignIns, Some(cursor), Seen::new(), now).await;
        let ids = sent(&sink);
        assert_eq!((ids.len(), ids[0].as_str(), ids[249].as_str()), (250, "s00000", "s00249"), "the overlap's and the new ones, oldest first, not the unsettled");
        assert_eq!(api.requested().len(), 3, "every page of the window");
        assert_eq!(run.outcome, Outcome::Collected { end_ms: now - SETTLE_MS, metrics: 250, points: 250, warning: None });
        assert!(sink.records.lock().unwrap().iter().all(|(_, h, doc)| h == &headers() && doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
        assert_eq!(run.seen.unwrap().len(), 250, "the ids within an overlap of the new cursor");
    }

    #[tokio::test]
    async fn late_events_in_the_overlap_are_sent_once() {
        let api = FakeGraph::default();
        let t0 = ms("2026-11-06T12:00:00Z");
        api.set("auditLogs/signIns", sign_ins("a", t0 - 120_000, 3));
        let sink = FakeSink::default();
        let first = sync(&api, &sink, &headers(), "example-org", &creds(), Stream::SignIns, Some(t0 - 600_000), Seen::new(), t0).await;
        let Outcome::Collected { end_ms, .. } = first.outcome else { panic!("{:?}", first.outcome) };
        // Graph shows one more, timed before the cursor, three minutes later.
        let mut events = sign_ins("a", t0 - 120_000, 3);
        events.push(fixtures::sign_in("late", &at(end_ms - 200_000), "jordan.patel@example.com", 0));
        api.set("auditLogs/signIns", events);
        let second = sync(&api, &sink, &headers(), "example-org", &creds(), Stream::SignIns, Some(end_ms), first.seen.unwrap(), t0 + 180_000).await;
        assert!(matches!(second.outcome, Outcome::Collected { metrics: 1, .. }), "{:?}", second.outcome);
        assert_eq!(sent(&sink), ["a00000", "a00001", "a00002", "late"]);
    }

    #[tokio::test]
    async fn a_window_with_too_many_pages_is_halved_until_it_fits_and_the_next_runs_read_on() {
        let api = FakeGraph { page: 10, ..Default::default() };
        let now = ms("2026-11-06T12:00:00Z");
        let cursor = now - 3_600_000;
        // 400 events in the window's first minutes: more than MAX_PAGES pages of 10.
        let burst = sign_ins("b", cursor + 1000, 400);
        api.set("auditLogs/signIns", burst.clone());
        let sink = FakeSink::default();
        let (mut cursor, mut seen) = (Some(cursor), Seen::new());
        for _ in 0..12 {
            let run = sync(&api, &sink, &headers(), "example-org", &creds(), Stream::SignIns, cursor, seen, now).await;
            let Outcome::Collected { end_ms, warning: None, .. } = run.outcome else { panic!("{:?}", run.outcome) };
            (cursor, seen) = (Some(end_ms), run.seen.unwrap());
        }
        assert_eq!(cursor, Some(now - SETTLE_MS));
        assert_eq!(sent(&sink), burst.iter().map(|e| e["id"].as_str().unwrap().to_string()).collect::<Vec<_>>(), "every event once, in order");
    }

    #[tokio::test]
    async fn a_missing_license_or_permission_names_it_and_publishes_nothing() {
        let premium = AzureError::Forbidden { action: None, message: "Authentication_RequestFromNonPremiumTenantOrB2CTenant: Neither tenant is B2C or tenant doesn't have premium license".into() };
        let api = FakeGraph { fail: Some(premium), ..Default::default() };
        let sink = FakeSink::default();
        let run = sync(&api, &sink, &headers(), "example-org", &creds(), Stream::SignIns, None, Seen::new(), ms("2026-11-06T12:00:00Z")).await;
        let Outcome::Failed(issue) = run.outcome else { panic!() };
        assert_eq!((issue.kind, issue.backoff, issue.missing_action.as_deref()), ("access_denied", Backoff::Interval, None));
        assert!(issue.message.starts_with("Reading sign-in logs needs Microsoft Entra ID P1 or P2"), "{}", issue.message);
        let denied = AzureError::Forbidden { action: None, message: "Authorization_RequestDenied: Insufficient privileges to complete the operation.".into() };
        let audits = issue_of(Stream::Audits, denied);
        assert_eq!((audits.kind, audits.missing_action.as_deref()), ("access_denied", Some("AuditLog.Read.All")));
        assert_eq!(issue_of(Stream::Risk, AzureError::Throttled("slow down".into())).backoff, Backoff::Exponential);
        assert!(sink.records.lock().unwrap().is_empty());
        let down = FakeSink { fail: true, ..Default::default() };
        let api = FakeGraph::default();
        api.set("auditLogs/directoryAudits", vec![fixtures::role_assigned("d-1", "2026-11-05T13:00:00Z", "Global Administrator")]);
        let run = sync(&api, &down, &headers(), "example-org", &creds(), Stream::Audits, None, Seen::new(), ms("2026-11-06T12:00:00Z")).await;
        assert!(matches!(run.outcome, Outcome::Failed(_)) && run.seen.is_none());
    }

    fn issue_of(stream: Stream, e: AzureError) -> Issue {
        issue(stream, &e)
    }

    #[tokio::test]
    async fn the_first_sync_reads_the_last_day_in_windows() {
        let api = FakeGraph::default();
        let now = ms("2026-11-06T12:00:00Z");
        api.set("identityProtection/riskDetections", vec![fixtures::risk("r-old", "2026-11-05T09:00:00Z", "high"), fixtures::risk("r-1", "2026-11-05T13:00:00Z", "high")]);
        let sink = FakeSink::default();
        let run = sync(&api, &sink, &headers(), "example-org", &creds(), Stream::Risk, None, Seen::new(), now).await;
        assert_eq!(sent(&sink), ["r-1"], "not before the lookback");
        assert!(matches!(run.outcome, Outcome::Collected { end_ms, .. } if end_ms == now - FIRST_SYNC_LOOKBACK_MS + MAX_WINDOW_MS));
        assert_eq!(Settings { sign_ins: true, audits: false, risk: true }.streams(), [Stream::SignIns, Stream::Risk]);
    }
}
