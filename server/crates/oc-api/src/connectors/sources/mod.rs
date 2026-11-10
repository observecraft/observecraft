//! Native sources: packages whose `native:` names code compiled in here, for vendors that aren't request/response
//! HTTP APIs (an SQS queue announcing S3 files, a WebSocket stream). The package keeps what is generic: the setup form
//! (sealed secrets), the streams, the record mapping (`record_id`, `filter`, `vars`, `output`, run by
//! `oc_connectors::engine::Records`), the catalog tile and the generic page with each stream's status. A [`Source`]
//! only reads: it tests a connection and runs a stream's *session*.
//!
//! Sessions run on the cloud crawlers' jobs (`cloud_crawl_jobs`, provider = package id, account = connection id,
//! scope = stream id), so one server holds a stream at a time (the lease, extended while it runs) and another takes
//! over when it stops. A session lasts [`SESSION`] then ends; the job is due again at once, so the stream is read
//! continuously and a server's restart costs at most a reconnect. Within a session:
//!
//! 1. records are mapped as they arrive and handed to [`Commit::commit`] in batches;
//! 2. a commit publishes the logs to Kafka with the org's headers and waits for the acknowledgement, then saves the
//!    stream's state (`connector_stream_state`); the source acknowledges its vendor (deletes the SQS message) only
//!    after that;
//! 3. a log's id is its record's identity (`_oc_ingest.id`), so a record read again after a crash between Kafka and
//!    the state is stored once.

pub mod crowdstrike_fdr;

use std::sync::Arc;
use std::time::{Duration, Instant};

use oc_connectors::engine::{self, Connection, Issue, Partition, Records, Tested};
use oc_connectors::manifest::{IssueKind, Manifest, Stream};
use serde_json::Value;

use super::{registry, runner, store};
use crate::cloud::jobs::{self, JobKey, MetricSink, Outcome};
use crate::state::AppState;

/// How long a session reads before its job is recorded and run again.
pub const SESSION: Duration = Duration::from_secs(240);
/// Logs per commit.
pub const BATCH: usize = 1_000;
/// When a stream's job is due again: right after a session, or after a failure (its backoff, capped at this).
const AGAIN_SECS: i64 = 1;
const RETRY_SECS: i64 = 900;

/// Where a session's batches go.
#[async_trait::async_trait]
pub trait Commit: Send {
    /// Publishes `logs` and, once they are acknowledged, saves `state` (None: kept as it is). An error means nothing
    /// may be acknowledged to the vendor.
    async fn commit(&mut self, logs: Vec<Value>, state: Option<&Partition>) -> Result<(), String>;
}

/// One stream of one connection, for one session.
pub struct Session<'a> {
    pub stream: &'a Stream,
    pub conn: &'a Connection,
    pub records: Records,
    /// The stream's saved state.
    pub state: Partition,
    pub deadline: Instant,
    pub out: &'a mut dyn Commit,
}

/// What a session did.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    /// Records read and logs written.
    pub records: usize,
    pub written: usize,
    /// Epoch ms the stream is read up to.
    pub until: Option<i64>,
    pub warning: Option<Issue>,
}

/// A native source.
#[async_trait::async_trait]
pub trait Source: Send + Sync {
    /// Checks the credentials and that the feed or stream answers.
    async fn test(&self, m: &Manifest, conn: &Connection) -> Tested;
    /// Reads a stream until the session's deadline (or the vendor ends it).
    async fn session(&self, s: Session<'_>) -> Result<Summary, Issue>;
}

/// The sources compiled into this server, by the name a package's `native:` gives.
pub const NAMES: [&str; 1] = [crowdstrike_fdr::NAME];

pub fn of(name: &str) -> Option<Arc<dyn Source>> {
    match name {
        crowdstrike_fdr::NAME => Some(Arc::new(crowdstrike_fdr::Fdr::aws())),
        _ => None,
    }
}

pub fn issue(kind: IssueKind, message: impl Into<String>) -> Issue {
    Issue { kind, message: message.into() }
}

pub fn failed(message: impl Into<String>) -> Tested {
    Tested { ok: false, message: message.into(), records: None }
}

/// The production commit: Kafka, then Postgres; every commit also extends the job's lease.
struct JobCommit<'a> {
    st: &'a AppState,
    sink: &'a dyn MetricSink,
    job: &'a JobKey,
    headers: Vec<(String, String)>,
}

#[async_trait::async_trait]
impl Commit for JobCommit<'_> {
    async fn commit(&mut self, logs: Vec<Value>, state: Option<&Partition>) -> Result<(), String> {
        for chunk in logs.chunks(runner::LOGS_PER_RECORD) {
            self.sink.publish(&self.job.org_id, &self.headers, serde_json::to_vec(chunk).unwrap_or_default()).await.map_err(|e| format!("Kafka: {e}"))?;
        }
        if let Some(p) = state {
            let mut s = engine::StreamState::default();
            s.partitions.insert(String::new(), p.clone());
            store::save_state(self.st, &self.job.org_id, &self.job.account_id, &self.job.scope, &s).await.map_err(|e| format!("saving the stream's state: {e}"))?;
        }
        jobs::extend(self.st, self.job).await.map_err(|e| format!("extending the job: {e}"))?;
        Ok(())
    }
}

/// Runs one session of a native package's stream: the connection read (its org's: the job's), unsealed, its state
/// loaded, the session run against Kafka.
pub async fn run_job(st: &AppState, sink: &dyn MetricSink, job: &JobKey) -> Result<Outcome, sqlx::Error> {
    let Some(source) = registry().package(&job.provider).and_then(|p| p.package.manifest.native.as_deref()).and_then(of) else { return Ok(Outcome::Idle) };
    run_with(st, sink, job, source.as_ref()).await
}

async fn run_with(st: &AppState, sink: &dyn MetricSink, job: &JobKey, source: &dyn Source) -> Result<Outcome, sqlx::Error> {
    let Some(pkg) = registry().package(&job.provider) else { return Ok(Outcome::Idle) };
    let m = &pkg.package.manifest;
    let Some(stream) = m.streams.iter().find(|s| s.id == job.scope) else { return Ok(Outcome::Idle) };
    let Some(saved) = store::get(st, &job.org_id, &m.id, &job.account_id).await? else { return Ok(Outcome::Idle) };
    if !saved.enabled || !saved.streams.contains(&stream.id) {
        return Ok(Outcome::Idle);
    }
    let mut conn = match store::unseal(st, &saved) {
        Ok(c) => c,
        Err(e) => return Ok(Outcome::Failed(runner::issue(&issue(IssueKind::Auth, e)))),
    };
    conn.config = engine::effective_config(m, &conn.config);
    let state = store::state(st, &job.org_id, &saved.id, &stream.id).await?.partitions.remove("").unwrap_or_default();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let records = match Records::new(m, stream, &conn, now_ms) {
        Ok(r) => r,
        Err(i) => return Ok(Outcome::Failed(runner::issue(&i))),
    };
    let headers = jobs::org_headers(st, &job.org_id).await?;
    let mut out = JobCommit { st, sink, job, headers };
    let session = Session { stream, conn: &conn, records, state, deadline: Instant::now() + SESSION, out: &mut out };
    Ok(match source.session(session).await {
        Ok(s) => Outcome::Collected { end_ms: s.until.or(job.collected_until).unwrap_or(now_ms), metrics: s.records, points: s.written, warning: s.warning.as_ref().map(runner::issue) },
        Err(i) => Outcome::Failed(runner::issue(&i)),
    })
}

/// The native packages' providers.
fn providers() -> Vec<String> {
    registry().packages.iter().filter(|p| p.package.manifest.native.as_deref().is_some_and(|n| NAMES.contains(&n))).map(|p| p.package.manifest.id.clone()).collect()
}

/// The loop: plans the native packages' jobs every minute and keeps up to `concurrency` sessions running, each in its
/// own task (a session lasts minutes, so they don't wait for each other).
pub async fn run(st: AppState, sink: Arc<dyn MetricSink>, concurrency: i64) {
    let providers = providers();
    if providers.is_empty() {
        return;
    }
    let mut planned_at: Option<Instant> = None;
    let mut running = tokio::task::JoinSet::new();
    loop {
        if planned_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            for id in &providers {
                let planned = match runner::desired(&st, id).await {
                    Ok(d) => jobs::plan(&st, id, &d).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = planned {
                    tracing::warn!(connector = %id, error = %e, "native connectors: planning failed");
                }
            }
            planned_at = Some(Instant::now());
        }
        while running.try_join_next().is_some() {}
        let free = concurrency - running.len() as i64;
        let due = if free > 0 {
            jobs::claim_any(&st, &providers, free).await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, "native connectors: claiming jobs failed");
                vec![]
            })
        } else {
            vec![]
        };
        if due.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        for job in due {
            let (st, sink) = (st.clone(), sink.clone());
            running.spawn(async move {
                let result = match run_job(&st, sink.as_ref(), &job).await {
                    Ok(outcome) => {
                        if let Outcome::Failed(i) = &outcome {
                            tracing::info!(connector = %job.provider, org = %job.org_id, stream = %job.scope, error = %i.message, "native connector session failed");
                        }
                        let again = if matches!(outcome, Outcome::Failed(_)) { RETRY_SECS } else { AGAIN_SECS };
                        jobs::record(&st, &job, &outcome, again).await
                    }
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    tracing::warn!(connector = %job.provider, error = %e, "native connectors: a session failed");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_native_package_has_its_source() {
        let natives: Vec<&str> = registry().packages.iter().filter_map(|p| p.package.manifest.native.as_deref()).collect();
        assert_eq!(natives, NAMES.to_vec(), "{natives:?}");
        assert!(NAMES.iter().all(|n| of(n).is_some()));
        assert!(providers().contains(&"crowdstrike-fdr".to_string()));
    }
}

#[cfg(test)]
pub mod testing {
    //! A commit recording what it was given, failing on demand.

    use super::*;

    #[derive(Default)]
    pub struct Recorder {
        pub logs: Vec<Value>,
        pub state: Option<Partition>,
        pub commits: usize,
        /// Commits that succeed before the next ones fail (None: none fails).
        pub fail_after: Option<usize>,
    }

    #[async_trait::async_trait]
    impl Commit for Recorder {
        async fn commit(&mut self, logs: Vec<Value>, state: Option<&Partition>) -> Result<(), String> {
            if self.fail_after.is_some_and(|n| self.commits >= n) {
                return Err("Kafka: broker unavailable".into());
            }
            self.commits += 1;
            self.logs.extend(logs);
            if let Some(s) = state {
                self.state = Some(s.clone());
            }
            Ok(())
        }
    }

    /// A package's connection and records, as a session gets them.
    pub fn records(id: &str, stream: &str, config: Value) -> (&'static Manifest, &'static Stream, Connection, Records) {
        let pkg = registry().package(id).unwrap();
        let m = &pkg.package.manifest;
        let s = m.streams.iter().find(|s| s.id == stream).unwrap();
        let conn = Connection { id: "0a1b2c3d-0000-4000-8000-00000000c0de".into(), name: id.into(), config: engine::effective_config(m, config.as_object().unwrap()), secrets: Default::default() };
        let records = Records::new(m, s, &conn, 1_794_312_000_000).unwrap();
        (m, s, conn, records)
    }
}
