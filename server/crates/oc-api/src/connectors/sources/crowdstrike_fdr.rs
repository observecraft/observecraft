//! CrowdStrike Falcon Data Replicator (`native: crowdstrike-fdr`, integrations/connectors/crowdstrike-fdr): Falcon
//! writes gzip NDJSON files to an S3 bucket and announces each batch on an SQS queue (`{cid, timestamp, bucket,
//! pathPrefix, files: [{path, size, checksum}]}`), with the feed's access key scoped to both.
//!
//! A session long-polls the queue (20 s waits, up to 10 notifications) until its deadline. For each notification it
//! streams every file from the connection's bucket (never the one the message names), decompressing as it reads and
//! splitting lines with a cap ([`MAX_LINE`]), so a file of any size holds at most one batch in memory. Each event is
//! mapped by the package's stream (its event-type filter, `s3_key`) and committed in batches (Kafka acknowledged);
//! the notification is deleted only after its last batch is, and its visibility extended while its files take long.
//! A notification whose processing fails stays on the queue and comes back: its events, read again, keep their ids
//! and are stored once. A file the bucket no longer has is skipped with a warning; a message that isn't a
//! notification (an S3 test event) is deleted.

use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aws_sdk_sqs::error::ProvideErrorMetadata;
use oc_connectors::engine::{Connection, Issue, Tested};
use oc_connectors::manifest::{IssueKind, Manifest};
use oc_connectors::output::Mapped;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use super::{failed, issue, Session, Source, Summary, BATCH};

pub const NAME: &str = "crowdstrike-fdr";
/// Long polling: the longest SQS allows.
const WAIT_SECS: u64 = 20;
/// How long a received notification is hidden from other readers; extended while its files are read.
const VISIBILITY_SECS: i32 = 300;
/// Longer lines are skipped (an FDR event is a few KiB).
pub const MAX_LINE: usize = 1 << 20;

/// A notification as received.
#[derive(Clone, Debug)]
pub struct Message {
    pub receipt: String,
    pub body: String,
}

/// An object's bytes as they arrive.
pub type Reader = Pin<Box<dyn AsyncBufRead + Send>>;

/// The feed: its queue and its bucket.
#[async_trait::async_trait]
pub trait Feed: Send + Sync {
    /// The notifications waiting (approximately).
    async fn waiting(&self) -> Result<u64, Issue>;
    async fn receive(&self, wait_secs: i32) -> Result<Vec<Message>, Issue>;
    /// Hides a notification for another visibility timeout.
    async fn keep(&self, receipt: &str) -> Result<(), Issue>;
    async fn delete(&self, receipt: &str) -> Result<(), Issue>;
    /// An object of the connection's bucket; `Ok(None)` when it isn't there.
    async fn object(&self, key: &str) -> Result<Option<Reader>, Issue>;
}

/// A connection's feed settings.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedConfig {
    pub queue_url: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_key: String,
}

impl FeedConfig {
    pub fn of(conn: &Connection) -> Result<FeedConfig, Issue> {
        let text = |map: &serde_json::Map<String, Value>, k: &str| map.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let queue_url = text(&conn.config, "sqs_queue_url").ok_or_else(|| issue(IssueKind::Other, "The SQS queue URL is missing"))?;
        let host = url::Url::parse(&queue_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        let region = host.strip_prefix("sqs.").and_then(|h| h.strip_suffix(".amazonaws.com")).filter(|r| !r.is_empty() && !r.contains('.')).ok_or_else(|| issue(IssueKind::Other, "The SQS queue URL names no region (https://sqs.<region>.amazonaws.com/…)"))?;
        Ok(FeedConfig {
            region: region.to_string(),
            queue_url,
            bucket: text(&conn.config, "s3_bucket").ok_or_else(|| issue(IssueKind::Other, "The S3 bucket is missing"))?,
            access_key_id: text(&conn.secrets, "aws_access_key_id").ok_or_else(|| issue(IssueKind::Auth, "The AWS access key id is missing"))?,
            secret_key: text(&conn.secrets, "aws_secret_key").ok_or_else(|| issue(IssueKind::Auth, "The AWS secret key is missing"))?,
        })
    }

    fn queue_name(&self) -> &str {
        self.queue_url.rsplit('/').next().unwrap_or_default()
    }
}

type Open = dyn Fn(&FeedConfig) -> Arc<dyn Feed> + Send + Sync;

/// The source: how a connection's feed is opened.
pub struct Fdr {
    open: Box<Open>,
}

impl Fdr {
    /// Through the AWS SDK.
    pub fn aws() -> Fdr {
        Fdr { open: Box::new(|c| Arc::new(AwsFeed::new(c, None))) }
    }

    #[cfg(test)]
    pub fn with(open: impl Fn(&FeedConfig) -> Arc<dyn Feed> + Send + Sync + 'static) -> Fdr {
        Fdr { open: Box::new(open) }
    }
}

#[async_trait::async_trait]
impl Source for Fdr {
    async fn test(&self, _m: &Manifest, conn: &Connection) -> Tested {
        let cfg = match FeedConfig::of(conn) {
            Ok(c) => c,
            Err(i) => return failed(i.message),
        };
        match (self.open)(&cfg).waiting().await {
            Ok(n) => Tested { ok: true, message: format!("Connected to the queue {}: about {n} notifications waiting (the bucket is read with the first one)", cfg.queue_name()), records: None },
            Err(i) => failed(i.message),
        }
    }

    async fn session(&self, mut s: Session<'_>) -> Result<Summary, Issue> {
        let cfg = FeedConfig::of(s.conn)?;
        let feed = (self.open)(&cfg);
        let mut sum = Summary { until: s.state.cursor, ..Default::default() };
        let mut problems = Problems::default();
        while let Some(left) = s.deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            let messages = feed.receive(left.as_secs().min(WAIT_SECS) as i32).await?;
            // A notification started is finished, past the deadline too (the lease is extended with each batch).
            for msg in messages {
                notification(feed.as_ref(), &cfg, &mut s, &msg, &mut sum, &mut problems).await?;
            }
        }
        sum.warning = problems.warning();
        Ok(sum)
    }
}

/// What a session skipped.
#[derive(Default)]
struct Problems {
    lines: usize,
    files: Vec<String>,
    messages: usize,
}

impl Problems {
    fn warning(&self) -> Option<Issue> {
        let mut parts = vec![];
        if self.lines > 0 {
            parts.push(format!("{} lines that aren't JSON events (or longer than 1 MiB) were skipped", self.lines));
        }
        if let Some(first) = self.files.first() {
            parts.push(format!("{} announced files weren't in the bucket (e.g. {first})", self.files.len()));
        }
        if self.messages > 0 {
            parts.push(format!("{} queue messages weren't FDR notifications and were deleted", self.messages));
        }
        (!parts.is_empty()).then(|| issue(IssueKind::Other, parts.join("; ")))
    }
}

/// Reads a notification's files and deletes it once their events are committed.
async fn notification(feed: &dyn Feed, cfg: &FeedConfig, s: &mut Session<'_>, msg: &Message, sum: &mut Summary, problems: &mut Problems) -> Result<(), Issue> {
    let body: Value = serde_json::from_str(&msg.body).unwrap_or(Value::Null);
    let Some(files) = body.get("files").and_then(Value::as_array) else {
        problems.messages += 1;
        return feed.delete(&msg.receipt).await;
    };
    let mut out = Mapped::default();
    let mut kept_at = Instant::now();
    for key in files.iter().filter_map(|f| f.get("path").and_then(Value::as_str)) {
        let Some(reader) = feed.object(key).await? else {
            problems.files.push(key.to_string());
            continue;
        };
        let mut reader = decompressed(reader).await.map_err(|e| issue(IssueKind::Other, format!("reading {key}: {e}")))?;
        let parent = |line: usize| json!({ "bucket": cfg.bucket, "key": key, "line": line });
        let (mut line, mut buf) = (0usize, Vec::new());
        loop {
            buf.clear();
            let (read, too_long) = next_line(&mut reader, &mut buf, MAX_LINE).await.map_err(|e| issue(IssueKind::Other, format!("reading {key}: {e}")))?;
            if read == 0 {
                break;
            }
            line += 1;
            if buf.iter().all(u8::is_ascii_whitespace) && !too_long {
                continue;
            }
            match serde_json::from_slice::<Value>(&buf) {
                Ok(record @ Value::Object(_)) if !too_long => {
                    sum.records += 1;
                    s.records.map(record, Some(&parent(line)), &mut out);
                }
                _ => problems.lines += 1,
            }
            if out.logs.len() >= BATCH {
                commit(s, &mut out, None, sum).await?;
                if kept_at.elapsed() >= Duration::from_secs(VISIBILITY_SECS as u64 / 3) {
                    feed.keep(&msg.receipt).await?;
                    kept_at = Instant::now();
                }
            }
        }
    }
    if let Some(at) = body.get("timestamp").and_then(Value::as_i64) {
        s.state.cursor = Some(s.state.cursor.map_or(at, |c| c.max(at)));
    }
    let state = s.state.clone();
    commit(s, &mut out, Some(&state), sum).await?;
    sum.until = s.state.cursor;
    feed.delete(&msg.receipt).await
}

async fn commit(s: &mut Session<'_>, out: &mut Mapped, state: Option<&oc_connectors::engine::Partition>, sum: &mut Summary) -> Result<(), Issue> {
    let logs = std::mem::take(&mut out.logs);
    let n = logs.len();
    s.out.commit(logs, state).await.map_err(|e| issue(IssueKind::Other, e))?;
    sum.written += n;
    Ok(())
}

/// Gzip (one or more members) when the object starts with gzip's magic bytes, else as it is.
async fn decompressed(mut reader: Reader) -> std::io::Result<Reader> {
    let head = reader.fill_buf().await?;
    if head.starts_with(&[0x1f, 0x8b]) {
        let mut d = async_compression::tokio::bufread::GzipDecoder::new(reader);
        d.multiple_members(true);
        Ok(Box::pin(tokio::io::BufReader::with_capacity(64 * 1024, d)))
    } else {
        Ok(reader)
    }
}

/// The next line into `buf` (with its newline): the bytes read (0 at the end) and whether it was longer than `max`
/// (then `buf` is left empty and the rest of the line skipped).
pub async fn next_line(r: &mut Reader, buf: &mut Vec<u8>, max: usize) -> std::io::Result<(usize, bool)> {
    let (mut read, mut over) = (0, false);
    loop {
        let chunk = r.fill_buf().await?;
        if chunk.is_empty() {
            return Ok((read, over));
        }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !over {
            if buf.len() + take > max {
                over = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        r.consume(take);
        read += take;
        if done {
            return Ok((read, over));
        }
    }
}

/// An AWS error as the stream's issue.
fn aws_issue(code: Option<&str>, message: String) -> Issue {
    let kind = match code.unwrap_or_default() {
        "InvalidClientTokenId" | "SignatureDoesNotMatch" | "InvalidAccessKeyId" | "UnrecognizedClientException" | "ExpiredToken" | "AuthFailure" | "MissingAuthenticationToken" => IssueKind::Auth,
        "AccessDenied" | "AccessDeniedException" | "AllAccessDisabled" => IssueKind::AccessDenied,
        "Throttling" | "ThrottlingException" | "RequestThrottled" | "SlowDown" | "RequestLimitExceeded" => IssueKind::Throttled,
        _ => IssueKind::Other,
    };
    let message = match kind {
        IssueKind::Auth => format!("AWS refused the feed's access key: {message}"),
        IssueKind::AccessDenied => format!("The feed's access key may not read this queue or bucket: {message}"),
        _ => message,
    };
    issue(kind, message)
}

fn sdk_issue<E: ProvideErrorMetadata + std::error::Error + 'static, R: std::fmt::Debug>(what: &str, e: aws_sdk_sqs::error::SdkError<E, R>) -> Issue {
    let code = e.code().map(str::to_string);
    let message = e.message().map(str::to_string).unwrap_or_else(|| aws_sdk_sqs::error::DisplayErrorContext(&e).to_string());
    if matches!(code.as_deref(), Some("AWS.SimpleQueueService.NonExistentQueue" | "QueueDoesNotExist")) {
        return issue(IssueKind::Other, "The SQS queue doesn't exist (check its URL)");
    }
    if code.as_deref() == Some("NoSuchBucket") {
        return issue(IssueKind::Other, "The S3 bucket doesn't exist (check its name)");
    }
    aws_issue(code.as_deref(), format!("{what}: {message}"))
}

/// The feed through the AWS SDK, signed with the connection's access key.
pub struct AwsFeed {
    sqs: aws_sdk_sqs::Client,
    s3: aws_sdk_s3::Client,
    queue_url: String,
    bucket: String,
}

impl AwsFeed {
    pub fn new(c: &FeedConfig, endpoint: Option<&str>) -> AwsFeed {
        use aws_sdk_sqs::config::{BehaviorVersion, Credentials, Region};
        let creds = Credentials::new(&c.access_key_id, &c.secret_key, None, None, "observecraft-crowdstrike-fdr");
        let mut sqs = aws_sdk_sqs::Config::builder().behavior_version(BehaviorVersion::latest()).region(Region::new(c.region.clone())).credentials_provider(creds.clone());
        let mut s3 = aws_sdk_s3::Config::builder().behavior_version(aws_sdk_s3::config::BehaviorVersion::latest()).region(aws_sdk_s3::config::Region::new(c.region.clone())).credentials_provider(creds);
        if let Some(e) = endpoint {
            sqs = sqs.endpoint_url(e);
            s3 = s3.endpoint_url(e).force_path_style(true);
        }
        AwsFeed { sqs: aws_sdk_sqs::Client::from_conf(sqs.build()), s3: aws_sdk_s3::Client::from_conf(s3.build()), queue_url: c.queue_url.clone(), bucket: c.bucket.clone() }
    }
}

#[async_trait::async_trait]
impl Feed for AwsFeed {
    async fn waiting(&self) -> Result<u64, Issue> {
        use aws_sdk_sqs::types::QueueAttributeName;
        let out = self.sqs.get_queue_attributes().queue_url(&self.queue_url).attribute_names(QueueAttributeName::ApproximateNumberOfMessages).send().await.map_err(|e| sdk_issue("SQS GetQueueAttributes", e))?;
        Ok(out.attributes().and_then(|a| a.get(&QueueAttributeName::ApproximateNumberOfMessages)).and_then(|n| n.parse().ok()).unwrap_or(0))
    }

    async fn receive(&self, wait_secs: i32) -> Result<Vec<Message>, Issue> {
        let out = self.sqs.receive_message().queue_url(&self.queue_url).max_number_of_messages(10).wait_time_seconds(wait_secs).visibility_timeout(VISIBILITY_SECS).send().await.map_err(|e| sdk_issue("SQS ReceiveMessage", e))?;
        Ok(out.messages().iter().filter_map(|m| Some(Message { receipt: m.receipt_handle()?.to_string(), body: m.body().unwrap_or_default().to_string() })).collect())
    }

    async fn keep(&self, receipt: &str) -> Result<(), Issue> {
        self.sqs.change_message_visibility().queue_url(&self.queue_url).receipt_handle(receipt).visibility_timeout(VISIBILITY_SECS).send().await.map_err(|e| sdk_issue("SQS ChangeMessageVisibility", e))?;
        Ok(())
    }

    async fn delete(&self, receipt: &str) -> Result<(), Issue> {
        self.sqs.delete_message().queue_url(&self.queue_url).receipt_handle(receipt).send().await.map_err(|e| sdk_issue("SQS DeleteMessage", e))?;
        Ok(())
    }

    async fn object(&self, key: &str) -> Result<Option<Reader>, Issue> {
        match self.s3.get_object().bucket(&self.bucket).key(key).send().await {
            Ok(o) => Ok(Some(Box::pin(o.body.into_async_read()))),
            Err(e) if e.as_service_error().is_some_and(|s| s.is_no_such_key()) => Ok(None),
            Err(e) => Err(sdk_issue("S3 GetObject", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::io::Write;
    use std::sync::Mutex;

    use super::super::testing::{records, Recorder};
    use super::*;

    const NOTIFICATION: &str = include_str!("../../../../../../integrations/connectors/crowdstrike-fdr/fixtures/notification.json");
    const DATA: &str = include_str!("../../../../../../integrations/connectors/crowdstrike-fdr/fixtures/data-part-00000.ndjson");
    const USERINFO: &str = include_str!("../../../../../../integrations/connectors/crowdstrike-fdr/fixtures/userinfo-part-00000.ndjson");
    const DATA_KEY: &str = "data/0b9e2a1c-4d5f-4a6b-8c7d-0e1f2a3b4c5d/part-00000.gz";
    const USERINFO_KEY: &str = "fdrv2/userinfo/0b9e2a1c-4d5f-4a6b-8c7d-0e1f2a3b4c5d/part-00000.gz";

    fn gzip(text: &str) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(text.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    /// A queue and a bucket in memory.
    #[derive(Default)]
    struct FakeFeed {
        queue: Mutex<VecDeque<Message>>,
        /// Received, not deleted.
        inflight: Mutex<Vec<Message>>,
        deleted: Mutex<Vec<String>>,
        objects: BTreeMap<String, Vec<u8>>,
        refuse: Option<Issue>,
    }

    impl FakeFeed {
        fn with(messages: &[&str], objects: &[(&str, Vec<u8>)]) -> FakeFeed {
            FakeFeed {
                queue: Mutex::new(messages.iter().enumerate().map(|(i, b)| Message { receipt: format!("r-{i}"), body: b.to_string() }).collect()),
                objects: objects.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
                ..Default::default()
            }
        }

        /// What wasn't deleted becomes visible again (the visibility timeout ran out).
        fn expire(&self) {
            let back: Vec<Message> = self.inflight.lock().unwrap().drain(..).collect();
            self.queue.lock().unwrap().extend(back);
        }
    }

    #[async_trait::async_trait]
    impl Feed for FakeFeed {
        async fn waiting(&self) -> Result<u64, Issue> {
            match &self.refuse {
                Some(i) => Err(i.clone()),
                None => Ok(self.queue.lock().unwrap().len() as u64),
            }
        }

        async fn receive(&self, _wait: i32) -> Result<Vec<Message>, Issue> {
            if let Some(i) = &self.refuse {
                return Err(i.clone());
            }
            let got: Vec<Message> = {
                let mut q = self.queue.lock().unwrap();
                let n = q.len().min(10);
                q.drain(..n).collect()
            };
            if got.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.inflight.lock().unwrap().extend(got.clone());
            Ok(got)
        }

        async fn keep(&self, _receipt: &str) -> Result<(), Issue> {
            Ok(())
        }

        async fn delete(&self, receipt: &str) -> Result<(), Issue> {
            self.inflight.lock().unwrap().retain(|m| m.receipt != receipt);
            self.deleted.lock().unwrap().push(receipt.to_string());
            Ok(())
        }

        async fn object(&self, key: &str) -> Result<Option<Reader>, Issue> {
            Ok(self.objects.get(key).map(|b| Box::pin(std::io::Cursor::new(b.clone())) as Reader))
        }
    }

    fn config() -> Value {
        json!({ "sqs_queue_url": "https://sqs.us-west-1.amazonaws.com/123456789012/cs-prod-cannon-queue", "s3_bucket": "cs-prod-cannon-0a1b2c3d-s3alias" })
    }

    fn secrets(c: &mut Connection) {
        c.secrets = json!({ "aws_access_key_id": "AKIAEXAMPLE000000001", "aws_secret_key": "fdr-test-secret" }).as_object().unwrap().clone();
    }

    async fn session(feed: Arc<FakeFeed>, config: Value, out: &mut Recorder, millis: u64) -> Result<Summary, Issue> {
        let (_, stream, mut conn, records) = records("crowdstrike-fdr", "events", config);
        secrets(&mut conn);
        let opened = feed.clone();
        let fdr = Fdr::with(move |c| {
            assert_eq!((c.region.as_str(), c.bucket.as_str()), ("us-west-1", "cs-prod-cannon-0a1b2c3d-s3alias"));
            opened.clone() as Arc<dyn Feed>
        });
        let s = Session { stream, conn: &conn, records, state: Default::default(), deadline: Instant::now() + Duration::from_millis(millis), out };
        fdr.session(s).await
    }

    fn feed() -> Arc<FakeFeed> {
        Arc::new(FakeFeed::with(&[NOTIFICATION], &[(DATA_KEY, gzip(DATA)), (USERINFO_KEY, gzip(USERINFO))]))
    }

    /// Every event of the announced files, shaped for the tile's pipeline; the notification deleted after its commit.
    #[tokio::test]
    async fn a_notifications_files_become_logs_and_it_is_deleted_after_they_are_committed() {
        let feed = feed();
        let mut out = Recorder::default();
        let sum = session(feed.clone(), config(), &mut out, 100).await.unwrap();
        assert_eq!((sum.records, sum.written, sum.until, sum.warning), (4, 4, Some(1_794_311_700_000), None));
        assert_eq!(*feed.deleted.lock().unwrap(), ["r-0"]);
        assert_eq!(out.state.as_ref().and_then(|s| s.cursor), Some(1_794_311_700_000));
        let logs = &out.logs;
        let first = &logs[0];
        assert_eq!(first["ddsource"], "crowdstrike-fdr");
        assert_eq!(first["event_simpleName"], "ProcessRollup2");
        assert_eq!(first["s3_key"], DATA_KEY, "the pipeline's dataset category reads s3_key");
        assert_eq!(first["host"], "STOREFRONT-WS-01");
        assert_eq!(first["message"], "ProcessRollup2 on STOREFRONT-WS-01: C:\\Windows\\System32\\cmd.exe /c whoami");
        assert_eq!(oc_connectors::expr::to_ms(&first["date"]), Some(1_794_311_640_030));
        assert_eq!(oc_connectors::expr::to_ms(&logs[1]["date"]), Some(1_794_311_641_120), "ContextTimeStamp (seconds) first");
        assert_eq!((logs[3]["UserName"].as_str(), logs[3]["s3_key"].as_str(), logs[3]["host"].as_str()), (Some("jane.doe"), Some(USERINFO_KEY), Some("cs-prod-cannon-queue")));
        assert!(!serde_json::to_string(logs).unwrap().contains("fdr-test-secret"));
    }

    /// Kafka down: nothing is deleted, the notification comes back and its events, read again, have the same ids.
    #[tokio::test]
    async fn a_failed_commit_keeps_the_notification_and_a_retry_writes_the_same_logs() {
        let feed = feed();
        let mut down = Recorder { fail_after: Some(0), ..Default::default() };
        let err = session(feed.clone(), config(), &mut down, 100).await.unwrap_err();
        assert!(err.message.contains("Kafka"), "{err:?}");
        assert!(feed.deleted.lock().unwrap().is_empty());
        feed.expire();
        let mut out = Recorder::default();
        session(feed.clone(), config(), &mut out, 100).await.unwrap();
        assert_eq!(*feed.deleted.lock().unwrap(), ["r-0"]);
        let ids = |r: &Recorder| r.logs.iter().map(|l| l["_oc_ingest"]["id"].clone()).collect::<Vec<_>>();
        let mut again = Recorder::default();
        feed.queue.lock().unwrap().push_back(Message { receipt: "r-9".into(), body: NOTIFICATION.into() });
        session(feed.clone(), config(), &mut again, 100).await.unwrap();
        assert_eq!(ids(&out), ids(&again), "the same events keep their ids: stored once");
    }

    /// The event-type filter keeps the listed events and the inventory records (which have no event type).
    #[tokio::test]
    async fn event_types_limit_the_volume() {
        let mut c = config();
        c["event_types"] = json!(["DnsRequest"]);
        let mut out = Recorder::default();
        let sum = session(feed(), c, &mut out, 100).await.unwrap();
        let kinds: Vec<&str> = out.logs.iter().map(|l| l["event_simpleName"].as_str().unwrap_or("-")).collect();
        assert_eq!(kinds, ["DnsRequest", "-"]);
        assert_eq!((sum.records, sum.written), (4, 2));
    }

    /// A large file is committed in batches as it streams; a missing file and bad lines are warnings; a message that
    /// isn't a notification is deleted.
    #[tokio::test]
    async fn large_files_stream_in_batches_and_problems_are_warnings() {
        let line = DATA.lines().next().unwrap();
        let mut big = String::new();
        for i in 0..(BATCH * 2 + 5) {
            big.push_str(&line.replace("0d6e1c2a-7f3b-11f0", &format!("{i:08x}-7f3b-11f0")));
            big.push('\n');
        }
        big.push_str("not json\n");
        big.push_str(&format!("{{\"pad\":\"{}\"}}\n", "x".repeat(MAX_LINE)));
        let body = NOTIFICATION.replace(USERINFO_KEY, "fdrv2/userinfo/gone.gz");
        let feed = Arc::new(FakeFeed::with(&[&body, r#"{"Service":"Amazon S3","Event":"s3:TestEvent"}"#], &[(DATA_KEY, gzip(&big))]));
        let mut out = Recorder::default();
        let sum = session(feed.clone(), config(), &mut out, 100).await.unwrap();
        assert_eq!(out.commits, 3, "two full batches, then the rest with the state");
        assert_eq!((sum.records, sum.written), (BATCH * 2 + 5, BATCH * 2 + 5));
        let w = sum.warning.unwrap().message;
        assert!(w.contains("2 lines") && w.contains("1 announced files") && w.contains("1 queue messages"), "{w}");
        assert_eq!(feed.deleted.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_test_reads_the_queue_and_names_refused_credentials() {
        let (m, _, mut conn, _) = records("crowdstrike-fdr", "events", config());
        secrets(&mut conn);
        let f = feed();
        assert!(Fdr::with(move |_| f.clone() as Arc<dyn Feed>).test(m, &conn).await.message.starts_with("Connected to the queue cs-prod-cannon-queue: about 1 notifications"));
        let refused = Arc::new(FakeFeed { refuse: Some(aws_issue(Some("InvalidClientTokenId"), "The security token included in the request is invalid.".into())), ..Default::default() });
        let t = Fdr::with(move |_| refused.clone() as Arc<dyn Feed>).test(m, &conn).await;
        assert!(!t.ok && t.message.starts_with("AWS refused the feed's access key"), "{t:?}");
        let mut bad = conn.clone();
        bad.config.insert("sqs_queue_url".into(), json!("https://example.com/123456789012/q"));
        assert!(FeedConfig::of(&bad).unwrap_err().message.contains("region"));
    }

    /// The AWS SDK against an in-process SQS and S3 (the JSON protocol, path-style objects): signed with the feed's
    /// key, long polling, visibility, deletion and streamed objects.
    #[tokio::test]
    async fn the_sdk_feed_speaks_sqs_and_s3() {
        use axum::extract::State as AxState;
        use axum::http::HeaderMap;
        #[derive(Default)]
        struct Aws {
            calls: Mutex<Vec<(String, Value)>>,
            auth: Mutex<Vec<String>>,
        }
        let aws = Arc::new(Aws::default());
        async fn sqs(AxState(aws): AxState<Arc<Aws>>, headers: HeaderMap, body: String) -> axum::response::Response {
            use axum::response::IntoResponse;
            let target = headers.get("x-amz-target").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            aws.auth.lock().unwrap().push(headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string());
            let req: Value = serde_json::from_str(&body).unwrap_or_default();
            aws.calls.lock().unwrap().push((target.clone(), req));
            let answer = match target.as_str() {
                // One notification, then an empty queue.
                "AmazonSQS.ReceiveMessage" if aws.calls.lock().unwrap().iter().filter(|(t, _)| t == "AmazonSQS.ReceiveMessage").count() == 1 => {
                    json!({ "Messages": [{ "MessageId": "m-1", "ReceiptHandle": "rh-1", "Body": NOTIFICATION }] })
                }
                "AmazonSQS.GetQueueAttributes" => json!({ "Attributes": { "ApproximateNumberOfMessages": "3" } }),
                _ => json!({}),
            };
            ([("content-type", "application/x-amz-json-1.0")], answer.to_string()).into_response()
        }
        async fn s3(axum::extract::Path((bucket, key)): axum::extract::Path<(String, String)>) -> axum::response::Response {
            use axum::response::IntoResponse;
            match (bucket.as_str(), key.as_str()) {
                ("cs-prod-cannon-0a1b2c3d-s3alias", DATA_KEY) => gzip(DATA).into_response(),
                _ => (axum::http::StatusCode::NOT_FOUND, [("content-type", "application/xml")], "<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>").into_response(),
            }
        }
        let app = axum::Router::new().route("/", axum::routing::post(sqs)).route("/{bucket}/{*key}", axum::routing::get(s3)).with_state(aws.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (m, stream, mut conn, records) = records("crowdstrike-fdr", "events", config());
        secrets(&mut conn);
        let e = endpoint.clone();
        let fdr = Fdr::with(move |c| Arc::new(AwsFeed::new(c, Some(&e))) as Arc<dyn Feed>);
        assert!(fdr.test(m, &conn).await.message.contains("about 3 notifications"));
        let mut out = Recorder::default();
        let s = Session { stream, conn: &conn, records, state: Default::default(), deadline: Instant::now() + Duration::from_millis(50), out: &mut out };
        let sum = fdr.session(s).await.unwrap();
        assert_eq!((sum.records, out.logs.len()), (3, 3), "the userinfo file isn't in this bucket");
        assert!(sum.warning.unwrap().message.contains("1 announced files"));
        let calls = aws.calls.lock().unwrap().clone();
        let receive = calls.iter().find(|(t, _)| t == "AmazonSQS.ReceiveMessage").unwrap();
        assert_eq!((receive.1["QueueUrl"].as_str(), receive.1["MaxNumberOfMessages"].as_i64(), receive.1["VisibilityTimeout"].as_i64()), (Some("https://sqs.us-west-1.amazonaws.com/123456789012/cs-prod-cannon-queue"), Some(10), Some(300)));
        let delete = calls.iter().find(|(t, _)| t == "AmazonSQS.DeleteMessage").unwrap();
        assert_eq!(delete.1["ReceiptHandle"], "rh-1");
        assert!(aws.auth.lock().unwrap().iter().all(|a| a.contains("Credential=AKIAEXAMPLE000000001/") && a.contains("/us-west-1/sqs/")));
    }
}
