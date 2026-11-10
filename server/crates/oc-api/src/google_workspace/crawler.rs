//! The Google Workspace poller, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider [`PROVIDER`]): one job per
//! connection and application (the job's scope is the application, its `collected_until` that application's cursor),
//! so several servers share them (`FOR UPDATE SKIP LOCKED`).
//!
//! A run ([`window`], then [`read_window`], every [`INTERVAL_SECS`]) reads an application's activities from a little
//! before the cursor ([`OVERLAP_MS`]) to [`SETTLE_MS`] ago; the first sync looks back [`FIRST_SYNC_LOOKBACK_MS`].
//! Google delivers some applications' activities late (login, token, groups and Drive by up to hours), so every
//! [`RESCAN_EVERY_MS`] a run reads back [`LATE_MS`] instead. Activities already sent are known by their identity
//! (application, `id.uniqueQualifier`, `id.time`: Postgres `google_workspace_seen`, pruned past the windows read
//! again), so a re-read sends only what arrived since.
//!
//! The Reports API answers newest first, page after page (`nextPageToken`). A window larger than a run's
//! [`MAX_PAGES`] is a backlog: the run sends what it read and leaves where it stopped ([`Backlog`]: the oldest time
//! read, that millisecond read again); the next runs carry on reading back from there to the window's start, and
//! only then does the cursor move to the window's end. No activity is skipped. Each event of an activity is a log
//! (JSON on `oc.logs.json`, with the org's headers), oldest first. A failed request publishes nothing, so the next
//! run reads the same pages again.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use super::api::{self, Credentials, WorkspaceApi, WorkspaceError, PAGE_SIZE};
use super::logs::{activity_hash, activity_ms, logs_of};
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::state::AppState;

pub const PROVIDER: &str = "google_workspace";
/// The Kafka topic the log ingester reads JSON logs from.
pub const LOGS_TOPIC: &str = "oc.logs.json";
/// How often an application is polled.
pub const INTERVAL_SECS: i64 = 300;
/// How far back the first sync goes.
pub const FIRST_SYNC_LOOKBACK_MS: i64 = 86_400_000;
/// Activities younger than this are left to the next run.
pub const SETTLE_MS: i64 = 30_000;
/// How far before the cursor every run reads again (activities recorded within minutes).
pub const OVERLAP_MS: i64 = 10 * 60_000;
/// How far before the cursor a re-read for late activities goes (Google documents lags of hours).
pub const LATE_MS: i64 = 4 * 3_600_000;
/// How often a run re-reads for late activities.
pub const RESCAN_EVERY_MS: i64 = 30 * 60_000;
/// Pages of activities read per run.
pub const MAX_PAGES: usize = 10;
/// Logs per Kafka record (well under the broker's 1 MiB message limit).
pub const LOGS_PER_RECORD: usize = 200;

/// A window read over several runs: its start, its end (where the cursor moves once it is read) and where the next
/// run carries on reading back from (epoch ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backlog {
    pub start_ms: i64,
    pub until_ms: i64,
    pub before_ms: i64,
}

/// What an application's runs keep between them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub backlog: Option<Backlog>,
    /// When a run last re-read for late activities.
    pub rescanned_ms: Option<i64>,
}

/// What a run reads: `start..=before`, the cursor moving to `until` once it reaches `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub start_ms: i64,
    pub until_ms: i64,
    pub before_ms: i64,
    /// Whether this run re-reads for late activities.
    pub rescan: bool,
}

/// The window of a run: the backlog's, else from the cursor (less the overlap, or the late window when a re-read is
/// due), never further back than the first sync's lookback.
pub fn window(cursor: Option<i64>, progress: &Progress, now_ms: i64) -> Window {
    let floor = now_ms - FIRST_SYNC_LOOKBACK_MS;
    let settled = now_ms - SETTLE_MS;
    if let Some(b) = progress.backlog {
        let start = b.start_ms.max(floor);
        if start < b.before_ms && b.before_ms <= b.until_ms && b.until_ms <= settled {
            return Window { start_ms: start, until_ms: b.until_ms, before_ms: b.before_ms, rescan: false };
        }
    }
    let Some(c) = cursor else { return Window { start_ms: floor, until_ms: settled, before_ms: settled, rescan: true } };
    let rescan = progress.rescanned_ms.is_none_or(|r| now_ms - r >= RESCAN_EVERY_MS);
    let start = (c.min(settled) - if rescan { LATE_MS } else { OVERLAP_MS }).max(floor);
    Window { start_ms: start, until_ms: settled, before_ms: settled, rescan }
}

/// A Google error as the job's issue.
pub fn issue(e: &WorkspaceError) -> Issue {
    let (kind, backoff) = match e {
        WorkspaceError::Throttled(_) => ("throttled", Backoff::Exponential),
        WorkspaceError::Auth(_) => ("auth", Backoff::Interval),
        WorkspaceError::Denied(_) => ("access_denied", Backoff::Interval),
        WorkspaceError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

/// A run's outcome, the backlog it leaves and the activities it sent (their key and time).
#[derive(Debug, PartialEq)]
pub struct Synced {
    pub outcome: Outcome,
    pub backlog: Option<Backlog>,
    pub sent: Vec<(i64, i64)>,
}

/// Reads `w` of `application`: see the module documentation. `cursor`: the job's `collected_until`; `seen`: the keys
/// of the activities already sent from `w.start_ms` on.
#[allow(clippy::too_many_arguments)]
pub async fn read_window(
    api: &dyn WorkspaceApi,
    sink: &dyn MetricSink,
    headers: &[(String, String)],
    org: &str,
    creds: &Credentials,
    application: &str,
    cursor: Option<i64>,
    w: &Window,
    seen: &HashSet<i64>,
) -> Synced {
    let kept = cursor.unwrap_or(w.start_ms).min(w.until_ms);
    let failed = |e: &WorkspaceError, backlog| Synced { outcome: Outcome::Failed(issue(e)), backlog, sent: vec![] };
    let previous = (w.before_ms != w.until_ms).then_some(Backlog { start_ms: w.start_ms, until_ms: w.until_ms, before_ms: w.before_ms });
    let mut read: Vec<(i64, Value)> = Vec::new();
    let (mut page_token, mut oldest, mut stopped_at) = (None::<String>, w.before_ms, None);
    for page in 1..=MAX_PAGES {
        let path = api::activities_path(application, w.start_ms, w.before_ms, page_token.as_deref(), PAGE_SIZE);
        let answer = match api.get(creds, &path).await {
            Ok(a) => a,
            Err(e) => return failed(&e, previous),
        };
        for a in answer["items"].as_array().into_iter().flatten() {
            let Some(at) = activity_ms(a) else { continue };
            oldest = oldest.min(at);
            read.push((at, a.clone()));
        }
        page_token = answer["nextPageToken"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
        if page_token.is_none() || oldest <= w.start_ms {
            break;
        }
        if page == MAX_PAGES {
            stopped_at = Some(oldest);
        }
    }
    let mut warning = None;
    if stopped_at == Some(w.before_ms) {
        // A run's pages within one millisecond: step past it.
        stopped_at = Some(w.before_ms - 1);
        warning = Some(Issue {
            kind: "other",
            message: format!("more than {} activities in one millisecond: the others of that millisecond were skipped", MAX_PAGES * PAGE_SIZE),
            missing_action: None,
            backoff: Backoff::Exponential,
        });
    }
    let mut fresh = HashSet::new();
    read.retain(|(at, a)| (w.start_ms..=w.before_ms).contains(at) && !seen.contains(&activity_hash(a)) && fresh.insert(activity_hash(a)));
    read.sort_by(|(t1, a), (t2, b)| t1.cmp(t2).then_with(|| a["id"]["uniqueQualifier"].as_str().cmp(&b["id"]["uniqueQualifier"].as_str())));
    let logs: Vec<Value> = read.iter().flat_map(|(_, a)| logs_of(a)).collect();
    for chunk in logs.chunks(LOGS_PER_RECORD) {
        let body = serde_json::to_vec(chunk).unwrap_or_default();
        if let Err(e) = sink.publish(org, headers, body).await {
            return failed(&WorkspaceError::Other(format!("Kafka: {e}")), previous);
        }
    }
    let sent = read.iter().map(|(at, a)| (activity_hash(a), *at)).collect();
    let n = logs.len();
    match stopped_at {
        None => Synced { outcome: Outcome::Collected { end_ms: w.until_ms, metrics: n, points: n, warning }, backlog: None, sent },
        Some(at) => Synced { outcome: Outcome::Collected { end_ms: kept, metrics: n, points: n, warning }, backlog: Some(Backlog { start_ms: w.start_ms, until_ms: w.until_ms, before_ms: at }), sent },
    }
}

/// The poller: Google through `api`, logs to `sink`.
pub struct WorkspaceCrawler {
    pub api: Arc<dyn WorkspaceApi>,
    pub sink: Arc<dyn MetricSink>,
}

/// An org's connection with its key unsealed: its id, credentials and applications.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<(String, Credentials, Vec<String>)>, sqlx::Error> {
    let row: Option<(String, String, String, Vec<String>)> =
        sqlx::query_as("SELECT id::text, key_enc, subject, applications FROM google_workspace_connections WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row.and_then(|(id, sealed, subject, applications)| {
        let key = api::parse_key(&st.secrets.decrypt(&sealed).ok()?).ok()?;
        Some((id, Credentials { key, subject }, applications))
    }))
}

/// An application's progress.
pub async fn progress(st: &AppState, org: &str, connection_id: &str, application: &str) -> Result<Progress, sqlx::Error> {
    type Row = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);
    let row: Option<Row> =
        sqlx::query_as("SELECT backlog_start_ms, backlog_until_ms, backlog_before_ms, rescanned_ms FROM google_workspace_progress WHERE org_id = $1 AND connection_id = $2::uuid AND application = $3")
            .bind(org)
            .bind(connection_id)
            .bind(application)
            .fetch_optional(&st.postgres)
            .await?;
    Ok(row.map_or_else(Progress::default, |(start, until, before, rescanned_ms)| Progress {
        backlog: match (start, until, before) {
            (Some(start_ms), Some(until_ms), Some(before_ms)) => Some(Backlog { start_ms, until_ms, before_ms }),
            _ => None,
        },
        rescanned_ms,
    }))
}

#[async_trait::async_trait]
impl Crawler for WorkspaceCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One job per connection and application.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String, Vec<String>)> =
            sqlx::query_as("SELECT c.org_id, c.id::text, c.applications FROM google_workspace_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().flat_map(|(org, id, apps)| apps.into_iter().map(move |a| (org.clone(), id.clone(), a))).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some((id, creds, applications)) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if id != job.account_id || !applications.contains(&job.scope) {
            return Ok(Outcome::Idle);
        }
        let (org, app) = (&job.org_id, &job.scope);
        let before = progress(st, org, &id, app).await?;
        let w = window(job.collected_until, &before, now_ms);
        let seen: Vec<i64> = sqlx::query_scalar("SELECT activity_key FROM google_workspace_seen WHERE org_id = $1 AND connection_id = $2::uuid AND application = $3 AND activity_ms >= $4")
            .bind(org)
            .bind(&id)
            .bind(app)
            .bind(w.start_ms)
            .fetch_all(&st.postgres)
            .await?;
        let headers = jobs::org_headers(st, org).await?;
        let synced = read_window(self.api.as_ref(), self.sink.as_ref(), &headers, org, &creds, app, job.collected_until, &w, &seen.into_iter().collect()).await;
        if !synced.sent.is_empty() {
            let (keys, times): (Vec<i64>, Vec<i64>) = synced.sent.iter().copied().unzip();
            sqlx::query(
                "INSERT INTO google_workspace_seen (org_id, connection_id, application, activity_key, activity_ms) SELECT $1, $2::uuid, $3, k, t FROM unnest($4::bigint[], $5::bigint[]) AS s(k, t) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(org)
            .bind(&id)
            .bind(app)
            .bind(&keys)
            .bind(&times)
            .execute(&st.postgres)
            .await?;
        }
        if matches!(synced.outcome, Outcome::Collected { .. }) {
            let after = Progress { backlog: synced.backlog, rescanned_ms: if w.rescan { Some(now_ms) } else { before.rescanned_ms } };
            let b = after.backlog;
            sqlx::query(
                "INSERT INTO google_workspace_progress (org_id, connection_id, application, backlog_start_ms, backlog_until_ms, backlog_before_ms, rescanned_ms) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7) \
                 ON CONFLICT (connection_id, application) DO UPDATE SET backlog_start_ms = EXCLUDED.backlog_start_ms, backlog_until_ms = EXCLUDED.backlog_until_ms, \
                 backlog_before_ms = EXCLUDED.backlog_before_ms, rescanned_ms = EXCLUDED.rescanned_ms",
            )
            .bind(org)
            .bind(&id)
            .bind(app)
            .bind(b.map(|b| b.start_ms))
            .bind(b.map(|b| b.until_ms))
            .bind(b.map(|b| b.before_ms))
            .bind(after.rescanned_ms)
            .execute(&st.postgres)
            .await?;
            // Activities older than any window still to be read again are no longer needed.
            let horizon = b.map_or(now_ms - LATE_MS - OVERLAP_MS, |b| b.start_ms).min(now_ms - LATE_MS - OVERLAP_MS) - 60_000;
            sqlx::query("DELETE FROM google_workspace_seen WHERE org_id = $1 AND connection_id = $2::uuid AND application = $3 AND activity_ms < $4")
                .bind(org)
                .bind(&id)
                .bind(app)
                .bind(horizon)
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
    use crate::google_workspace::fixtures::{self, FakeReports};

    fn ms(t: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp_millis()
    }

    fn at(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    fn headers() -> Vec<(String, String)> {
        vec![("x-oc-org".into(), "example-org".into())]
    }

    fn published(sink: &FakeSink) -> Vec<Value> {
        sink.records.lock().unwrap().iter().flat_map(|(_, _, doc)| doc.as_array().unwrap().clone()).collect()
    }

    fn uniques(sink: &FakeSink) -> Vec<String> {
        published(sink).iter().map(|l| l["id"]["uniqueQualifier"].as_str().unwrap().to_string()).collect()
    }

    /// `n` logins `step` ms apart from `from` on, unique qualifiers `{prefix}{i}`.
    fn logins(prefix: &str, from: i64, n: usize, step: i64) -> Vec<Value> {
        (0..n).map(|i| fixtures::login_success(&format!("{prefix}{i:05}"), &at(from + i as i64 * step))).collect()
    }

    /// Runs `app` as the job would: the window, the seen activities and the progress kept in memory.
    struct Runner {
        cursor: Option<i64>,
        progress: Progress,
        seen: Vec<(i64, i64)>,
    }

    impl Runner {
        fn new() -> Self {
            Runner { cursor: None, progress: Progress::default(), seen: vec![] }
        }

        async fn run(&mut self, api: &FakeReports, sink: &FakeSink, app: &str, now: i64) -> Synced {
            let w = window(self.cursor, &self.progress, now);
            let seen = self.seen.iter().filter(|(_, t)| *t >= w.start_ms).map(|(k, _)| *k).collect();
            let synced = read_window(api, sink, &headers(), "example-org", &fixtures::credentials(), app, self.cursor, &w, &seen).await;
            if let Outcome::Collected { end_ms, .. } = synced.outcome {
                self.cursor = Some(end_ms);
                self.progress = Progress { backlog: synced.backlog, rescanned_ms: if w.rescan { Some(now) } else { self.progress.rescanned_ms } };
                self.seen.extend(synced.sent.iter().copied());
            }
            synced
        }
    }

    #[test]
    fn windows_overlap_the_cursor_and_reread_late_activities_every_half_hour() {
        let now = ms("2026-11-06T12:00:00Z");
        let first = window(None, &Progress::default(), now);
        assert_eq!(first, Window { start_ms: now - FIRST_SYNC_LOOKBACK_MS, until_ms: now - SETTLE_MS, before_ms: now - SETTLE_MS, rescan: true });
        let cursor = now - 300_000;
        let fresh = Progress { backlog: None, rescanned_ms: Some(now - 600_000) };
        assert_eq!(window(Some(cursor), &fresh, now).start_ms, cursor - OVERLAP_MS);
        let due = Progress { backlog: None, rescanned_ms: Some(now - RESCAN_EVERY_MS) };
        assert_eq!(window(Some(cursor), &due, now), Window { start_ms: cursor - LATE_MS, until_ms: now - SETTLE_MS, before_ms: now - SETTLE_MS, rescan: true });
        assert_eq!(window(Some(now - 30 * 86_400_000), &fresh, now).start_ms, now - FIRST_SYNC_LOOKBACK_MS, "never past the lookback");
        let backlog = Progress { backlog: Some(Backlog { start_ms: now - 7_200_000, until_ms: now - 60_000, before_ms: now - 3_600_000 }), rescanned_ms: None };
        assert_eq!(window(Some(cursor), &backlog, now), Window { start_ms: now - 7_200_000, until_ms: now - 60_000, before_ms: now - 3_600_000, rescan: false });
    }

    #[tokio::test]
    async fn the_first_sync_sends_a_days_activities_oldest_first() {
        let now = ms("2026-11-06T12:00:00Z");
        let mut log = logins("old", ms("2026-11-05T10:00:00Z"), 5, 1000);
        log.extend(logins("a", ms("2026-11-05T13:00:00Z"), 2500, 30_000));
        log.push(fixtures::login_failure("unsettled", &at(now - 5_000), "jordan.patel@example.com"));
        log.push(fixtures::role_assigned("admin-1", &at(now - 600_000)));
        let api = FakeReports::with(log);
        let sink = FakeSink::default();
        let mut r = Runner::new();
        let synced = r.run(&api, &sink, "login", now).await;
        let sent = uniques(&sink);
        assert_eq!(sent.len(), 2500, "the day's settled logins, without the day before, the unsettled or another application");
        assert_eq!((sent[0].as_str(), sent.last().unwrap().as_str()), ("a00000", "a02499"), "oldest first");
        assert_eq!(api.requested().len(), 3, "pages of 1000, 1000 and 500");
        assert!(api.requested()[1].contains("&pageToken="));
        assert!(published(&sink).iter().all(|l| l["ddsource"] == "gsuite" && l["service"] == "login"));
        assert_eq!(sink.records.lock().unwrap()[0].1, headers(), "with the org's headers");
        assert!(sink.records.lock().unwrap().iter().all(|(_, _, doc)| doc.as_array().unwrap().len() <= LOGS_PER_RECORD));
        assert_eq!((synced.outcome, synced.backlog, synced.sent.len()), (Outcome::Collected { end_ms: now - SETTLE_MS, metrics: 2500, points: 2500, warning: None }, None, 2500));
    }

    #[tokio::test]
    async fn the_overlap_read_again_sends_only_new_activities() {
        let now = ms("2026-11-06T12:00:00Z");
        let api = FakeReports::with(logins("a", now - 3_600_000, 50, 60_000));
        let sink = FakeSink::default();
        let mut r = Runner::new();
        r.run(&api, &sink, "login", now).await;
        assert_eq!(uniques(&sink).len(), 50);
        api.add(logins("b", now + 10_000, 3, 1_000));
        let next = r.run(&api, &sink, "login", now + 300_000).await;
        assert!(api.requested().last().unwrap().contains(&format!("startTime={}", api::rfc3339_ms(now - SETTLE_MS - OVERLAP_MS).replace(':', "%3A"))));
        assert_eq!(uniques(&sink).len(), 53, "the overlap's activities aren't sent twice");
        assert_eq!(next.sent.len(), 3);
    }

    #[tokio::test]
    async fn activities_arriving_hours_late_are_sent_by_the_next_reread() {
        let now = ms("2026-11-06T12:00:00Z");
        let api = FakeReports::with(logins("a", now - 7_200_000, 10, 60_000));
        let sink = FakeSink::default();
        let mut r = Runner::new();
        r.run(&api, &sink, "login", now).await;
        // An activity of two hours ago shows up now: a run within the half hour doesn't read that far back…
        api.add(vec![fixtures::login_failure("late", &at(now - 7_000_000), "sam.lee@example.com")]);
        let quick = r.run(&api, &sink, "login", now + 300_000).await;
        assert!(quick.sent.is_empty());
        // …the next re-read does, and sends it alone, once.
        let mut t = now + 300_000;
        while t < now + RESCAN_EVERY_MS {
            t += 300_000;
            r.run(&api, &sink, "login", t).await;
        }
        let sent = uniques(&sink);
        assert_eq!(sent.iter().filter(|u| *u == "late").count(), 1);
        assert_eq!(sent.len(), 11);
    }

    #[tokio::test]
    async fn a_backlog_past_a_runs_pages_is_read_on_by_the_next_runs_without_gaps() {
        let now = ms("2026-11-06T12:00:00Z");
        let total = MAX_PAGES * PAGE_SIZE * 2 + 350;
        // Several activities a millisecond, so pages end inside a millisecond.
        let burst: Vec<Value> = (0..total).map(|i| fixtures::login_success(&format!("u{i:06}"), &at(ms("2026-11-06T06:00:00Z") + (i / 3) as i64))).collect();
        let api = FakeReports::with(burst);
        let sink = FakeSink::default();
        let mut r = Runner::new();
        let mut runs = 0;
        loop {
            runs += 1;
            let synced = r.run(&api, &sink, "login", now).await;
            let Outcome::Collected { warning: None, .. } = synced.outcome else { panic!("{:?}", synced.outcome) };
            if synced.backlog.is_none() {
                break;
            }
            assert_eq!(r.cursor, Some(now - FIRST_SYNC_LOOKBACK_MS), "the cursor waits for the backlog");
        }
        assert_eq!(runs, 3);
        assert_eq!(r.cursor, Some(now - SETTLE_MS), "then moves to the backlog's end");
        let mut sent = uniques(&sink);
        assert_eq!(sent.len(), total, "nothing sent twice at the pages' edges");
        sent.sort();
        sent.dedup();
        assert_eq!(sent.len(), total, "every activity of the window");
    }

    #[tokio::test]
    async fn a_refused_key_or_kafka_failure_publishes_nothing_and_keeps_the_backlog() {
        let now = ms("2026-11-06T12:00:00Z");
        let refused = FakeReports { refuse: true, ..Default::default() };
        let sink = FakeSink::default();
        let progress = Progress { backlog: Some(Backlog { start_ms: now - 7_200_000, until_ms: now - 60_000, before_ms: now - 3_600_000 }), rescanned_ms: None };
        let w = window(Some(now - 86_000_000), &progress, now);
        let synced = read_window(&refused, &sink, &headers(), "example-org", &fixtures::credentials(), "login", Some(now - 86_000_000), &w, &HashSet::new()).await;
        match &synced.outcome {
            Outcome::Failed(issue) => assert_eq!((issue.kind, issue.backoff), ("access_denied", Backoff::Interval)),
            other => panic!("{other:?}"),
        }
        assert_eq!(synced.backlog, progress.backlog, "the backlog is kept");
        assert!(sink.records.lock().unwrap().is_empty() && synced.sent.is_empty());
        let api = FakeReports::with(vec![fixtures::login_success("1", &at(now - 60_000))]);
        let down = FakeSink { fail: true, ..Default::default() };
        let w = window(None, &Progress::default(), now);
        let synced = read_window(&api, &down, &headers(), "example-org", &fixtures::credentials(), "login", None, &w, &HashSet::new()).await;
        assert!(matches!(synced.outcome, Outcome::Failed(_)) && synced.sent.is_empty(), "unsent activities aren't marked seen");
    }

    #[tokio::test]
    async fn jobs_keep_their_progress_and_seen_activities_in_postgres() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let admin = crate::test_support::admin_of_new_org(st, "google-workspace-crawler").await;
        let org = admin.org_id.clone();
        sqlx::query("INSERT INTO google_workspace_connections (org_id, client_email, key_enc, subject, applications) VALUES ($1, 'sa@example-project.iam.gserviceaccount.com', $2, 'admin@example.com', '{login}')")
            .bind(&org)
            .bind(st.secrets.encrypt(&fixtures::key_json()))
            .execute(&st.postgres)
            .await
            .unwrap();
        let (id, _, apps) = connection(st, &org).await.unwrap().unwrap();
        assert_eq!(apps, ["login"]);
        let now = chrono::Utc::now().timestamp_millis();
        let api = Arc::new(FakeReports::with(logins("a", now - 3_600_000, 20, 60_000)));
        let sink = Arc::new(FakeSink::default());
        let crawler = WorkspaceCrawler { api: api.clone(), sink: sink.clone() };
        assert_eq!(crawler.desired(st).await.unwrap().into_iter().filter(|(o, _, _)| *o == org).collect::<Vec<_>>(), [(org.clone(), id.clone(), "login".to_string())]);
        let job = JobKey { provider: PROVIDER.into(), org_id: org.clone(), account_id: id.clone(), scope: "login".into(), collected_until: None };
        let Outcome::Collected { end_ms, metrics: 20, .. } = crawler.run(st, &job, now).await.unwrap() else { panic!() };
        assert_eq!(progress(st, &org, &id, "login").await.unwrap().rescanned_ms, Some(now));
        // The next run reads the overlap again and sends nothing twice.
        let job = JobKey { collected_until: Some(end_ms), ..job };
        assert!(matches!(crawler.run(st, &job, now + 300_000).await.unwrap(), Outcome::Collected { metrics: 0, .. }));
        assert_eq!(uniques(&sink).len(), 20);
        // Another application or connection isn't this job's.
        assert_eq!(crawler.run(st, &JobKey { scope: "drive".into(), ..job.clone() }, now).await.unwrap(), Outcome::Idle);
        let other: i64 = sqlx::query_scalar("SELECT count(*) FROM google_workspace_seen WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(other, 20);
    }
}
