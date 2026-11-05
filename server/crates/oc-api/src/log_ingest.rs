//! The Kafka log ingester (`OC_LOG_INGEST=kafka`): the server reads the log topics itself, normalizes each log
//! (`log_normalize`, as Vector's `normalize.vrl` did), writes the 72-hour copy (`oc.logs_raw_dedup`), runs the
//! indexer (`logproc::index`: pipelines, indexes, quotas, generated metrics, forwarding) and publishes live tail
//! (`oc.logs.live`). The Collector and Vector are then off the log path.
//!
//! Servers share the partitions through Postgres (`log_ingest_partitions`): each holds a lease on its fair
//! share, renewed every few seconds, and moves a partition's offset only after its logs are stored, so a
//! partition another server takes over (a restart, a scale-out) is read again from there. Delivery is
//! at least once; stored rows are deduplicated by id (`oc.logs_dedup` is a ReplacingMergeTree).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use rskafka::client::partition::{Compression, OffsetAt, PartitionClient, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder};
use rskafka::record::Record;
use sqlx::PgPool;
use tokio::task::JoinHandle;

use crate::log_normalize::{self, Row};
use crate::log_records::{self, Topic};
use crate::logproc::{self, Indexer};
use crate::state::AppState;

/// How long a lease lasts without renewal.
const LEASE: Duration = Duration::from_secs(30);
/// How often leases are renewed and shares rebalanced.
const SYNC_EVERY: Duration = Duration::from_secs(5);
/// Largest fetch from one partition (compressed bytes; an OTLP batch expands several times when decoded).
const FETCH_BYTES: i32 = 1024 * 1024;
/// Batches decoded and being stored at once, across partitions (`OC_LOG_INGEST_CONCURRENCY`, default 6):
/// bounds the server's memory to about 200 MiB each.
static STORE_SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(std::env::var("OC_LOG_INGEST_CONCURRENCY").ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(6)));
/// `OC_LOG_INGEST_RAW=false` skips the 72-hour copy of the logs no index keeps (`oc.logs_raw_dedup`; archives
/// read it with the indexed logs).
static RAW_COPY: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("OC_LOG_INGEST_RAW").map_or(true, |v| v != "false"));
/// The Collector's topic for other signals carried as OTLP logs (`oc.signal`).
const SIGNALS_TOPIC: &str = "oc.otlp.signals";
/// How often the orgs with a live tail session are read again.
const WATCHED_EVERY: Duration = Duration::from_secs(2);
/// Live tail batches being published at once; more are dropped, as Vector's `drop_newest` buffer did.
const LIVE_IN_FLIGHT: usize = 8;
/// How long a fetch waits for records.
const FETCH_WAIT_MS: i32 = 500;
/// A batch is stored once its records reach this size (decompressed)…
const BATCH_BYTES: usize = 24 * 1024 * 1024;
/// …or once its first record waited this long: larger inserts make fewer parts for ClickHouse to merge.
const BATCH_WAIT: Duration = Duration::from_secs(1);

/// Partitions each of `owners` servers holds when `total` are shared.
fn fair_share(total: usize, owners: usize) -> usize {
    total.div_ceil(owners.max(1))
}

/// This server's leases on the log topics' partitions.
pub struct Leases {
    owner: String,
}

impl Leases {
    pub fn new() -> Self {
        Leases { owner: format!("{:032x}", rand::random::<u128>()) }
    }

    /// Records the topics' partitions, renews this server's leases, takes free or expired ones up to its share
    /// and hands back those beyond it. Returns the partitions held, with their next offsets.
    pub async fn sync(&self, pg: &PgPool, partitions: &[(String, i32)]) -> Result<Vec<(String, i32, Option<i64>)>, sqlx::Error> {
        let (topics, numbers): (Vec<String>, Vec<i32>) = partitions.iter().cloned().unzip();
        sqlx::query("INSERT INTO log_ingest_partitions (topic, partition) SELECT * FROM UNNEST($1::text[], $2::int[]) ON CONFLICT DO NOTHING")
            .bind(&topics)
            .bind(&numbers)
            .execute(pg)
            .await?;
        let lease = LEASE.as_secs_f64();
        sqlx::query("UPDATE log_ingest_partitions SET lease_until = now() + make_interval(secs => $2) WHERE owner = $1 AND topic = ANY($3)")
            .bind(&self.owner)
            .bind(lease)
            .bind(&topics)
            .execute(pg)
            .await?;
        sqlx::query(
            "INSERT INTO log_ingest_servers (owner, topics, seen_until) VALUES ($1, $2, now() + make_interval(secs => $3))
             ON CONFLICT (owner) DO UPDATE SET topics = EXCLUDED.topics, seen_until = EXCLUDED.seen_until",
        )
        .bind(&self.owner)
        .bind(&topics)
        .bind(lease)
        .execute(pg)
        .await?;
        let servers: i64 = sqlx::query_scalar("SELECT count(*) FROM log_ingest_servers WHERE topics && $1 AND seen_until > now()").bind(&topics).fetch_one(pg).await?;
        let share = fair_share(partitions.len(), servers as usize) as i64;
        let held: i64 = sqlx::query_scalar("SELECT count(*) FROM log_ingest_partitions WHERE owner = $1 AND topic = ANY($2) AND lease_until > now()")
            .bind(&self.owner)
            .bind(&topics)
            .fetch_one(pg)
            .await?;
        if held < share {
            sqlx::query(
                "UPDATE log_ingest_partitions p SET owner = $1, lease_until = now() + make_interval(secs => $2) FROM (
                   SELECT topic, partition FROM log_ingest_partitions WHERE topic = ANY($3) AND (owner IS NULL OR lease_until <= now())
                   ORDER BY partition, topic LIMIT $4 FOR UPDATE SKIP LOCKED) free
                 WHERE p.topic = free.topic AND p.partition = free.partition",
            )
            .bind(&self.owner)
            .bind(lease)
            .bind(&topics)
            .bind(share - held)
            .execute(pg)
            .await?;
        } else if held > share {
            sqlx::query(
                "UPDATE log_ingest_partitions p SET owner = NULL, lease_until = 'epoch' FROM (
                   SELECT topic, partition FROM log_ingest_partitions WHERE owner = $1 AND topic = ANY($2)
                   ORDER BY partition DESC, topic DESC LIMIT $3 FOR UPDATE) extra
                 WHERE p.topic = extra.topic AND p.partition = extra.partition",
            )
            .bind(&self.owner)
            .bind(&topics)
            .bind(held - share)
            .execute(pg)
            .await?;
        }
        sqlx::query_as("SELECT topic, partition, next_offset FROM log_ingest_partitions WHERE owner = $1 AND topic = ANY($2) AND lease_until > now() ORDER BY topic, partition")
            .bind(&self.owner)
            .bind(&topics)
            .fetch_all(pg)
            .await
    }

    /// Moves a partition's offset; false when this server no longer holds it.
    pub async fn commit(&self, pg: &PgPool, topic: &str, partition: i32, next_offset: i64) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("UPDATE log_ingest_partitions SET next_offset = $4 WHERE topic = $1 AND partition = $2 AND owner = $3 AND lease_until > now()")
            .bind(topic)
            .bind(partition)
            .bind(&self.owner)
            .bind(next_offset)
            .execute(pg)
            .await?;
        Ok(done.rows_affected() == 1)
    }
}

/// The rows of a fetch's records, and the other signals some of them carried (`(org, record)` for
/// `oc.otlp.signals`, with the record's key and headers): decoding and normalization, off the async workers.
fn rows_of(topic: Topic, records: Vec<Record>) -> (Vec<Row>, Vec<(String, Record)>) {
    let now = chrono::Utc::now();
    let (mut rows, mut signals) = (Vec::new(), Vec::new());
    for record in records {
        match log_records::decode(topic, &record) {
            Ok((events, passed)) => {
                rows.extend(events.into_iter().filter_map(|e| log_normalize::normalize(e, now)));
                if let Some(value) = passed {
                    let org = record.headers.get("x-oc-org").map(|o| String::from_utf8_lossy(o).into_owned()).unwrap_or_default();
                    signals.push((org, Record { value: Some(value), ..record }));
                }
            }
            Err(e) => tracing::warn!(error = %e, "skipping an undecodable log record"),
        }
    }
    (rows, signals)
}

/// Stores rows: the 72-hour copy and the indexed rows together, then live tail (best effort).
async fn store(st: &AppState, ix: &Arc<Indexer>, out: &Arc<Outputs>, rows: Vec<Row>) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp_millis();
    let (ix_rows, watched) = (ix.clone(), out.watched());
    let (rows, by_org) = tokio::task::spawn_blocking(move || {
        let mut rows = rows;
        // Live tail gets the logs as normalized, before the org's pipelines (as from Vector).
        let by_org = live_payloads(&rows, &watched);
        logproc::prepare_normalized(&ix_rows, &mut rows, now);
        (rows, by_org)
    })
    .await
    .map_err(|e| e.to_string())?;
    // The 72-hour copy holds what no index keeps, processed: archives read it with the indexed logs.
    let unindexed = logproc::index(st, ix, rows, *RAW_COPY).await.map_err(|e| format!("indexing logs: {e}"))?;
    if !unindexed.is_empty() {
        logproc::insert_rows(st, "oc.logs_raw_dedup", unindexed).await.map_err(|e| format!("storing unindexed logs: {e}"))?;
    }
    if !by_org.is_empty() {
        out.publish(by_org);
    }
    Ok(())
}

/// Live tail's records: the logs of orgs with an open session, as normalized, grouped by org.
fn live_payloads(rows: &[Row], watched: &HashSet<String>) -> HashMap<String, Vec<Vec<u8>>> {
    let mut by_org: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
    if watched.is_empty() {
        return by_org;
    }
    for row in rows.iter().filter(|r| watched.contains(&r.org_id)) {
        if let Ok(json) = serde_json::to_vec(row) {
            by_org.entry(row.org_id.clone()).or_default().push(json);
        }
    }
    by_org
}

/// Records for one topic, keyed by org on the org's partition (`live_feed::partition_of`, as Vector's sinks keyed them).
struct TopicProducer {
    client: Arc<Client>,
    topic: &'static str,
    partitions: tokio::sync::OnceCell<i32>,
    clients: tokio::sync::Mutex<HashMap<i32, Arc<PartitionClient>>>,
}

impl TopicProducer {
    fn new(client: Arc<Client>, topic: &'static str) -> Self {
        TopicProducer { client, topic, partitions: Default::default(), clients: Default::default() }
    }

    /// Writes `(org, record)`s and waits for Kafka's acknowledgement.
    async fn produce(&self, records: Vec<(String, Record)>) -> Result<(), String> {
        let count = *self
            .partitions
            .get_or_try_init(|| async {
                let topics = self.client.list_topics().await.map_err(|e| e.to_string())?;
                topics.iter().find(|t| t.name == self.topic).map(|t| t.partitions.len() as i32).ok_or_else(|| format!("Kafka topic {} doesn't exist", self.topic))
            })
            .await?;
        let mut per_partition: HashMap<i32, Vec<Record>> = HashMap::new();
        for (org, record) in records {
            per_partition.entry(crate::live_feed::partition_of(&org, count)).or_default().push(record);
        }
        for (partition, records) in per_partition {
            let pc = {
                let mut clients = self.clients.lock().await;
                match clients.get(&partition) {
                    Some(pc) => pc.clone(),
                    None => {
                        let pc = Arc::new(self.client.partition_client(self.topic, partition, UnknownTopicHandling::Retry).await.map_err(|e| e.to_string())?);
                        clients.insert(partition, pc.clone());
                        pc
                    }
                }
            };
            pc.produce(records, Compression::Zstd).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// What the ingester writes back to Kafka: live tail (`oc.logs.live`, best effort) and the other signals some
/// senders put on the logs topics (`oc.otlp.signals`, for the Collector, before the offset moves).
struct Outputs {
    client: Arc<Client>,
    live: TopicProducer,
    signals: TopicProducer,
    /// Orgs with an open live tail session (`live_feed::watched`): only their logs are published.
    watched: std::sync::RwLock<Arc<HashSet<String>>>,
    in_flight: Arc<tokio::sync::Semaphore>,
}

impl Outputs {
    fn new(client: Arc<Client>) -> Self {
        Outputs {
            live: TopicProducer::new(client.clone(), crate::live_feed::TOPIC),
            signals: TopicProducer::new(client.clone(), SIGNALS_TOPIC),
            client,
            watched: Default::default(),
            in_flight: Arc::new(tokio::sync::Semaphore::new(LIVE_IN_FLIGHT)),
        }
    }

    fn watched(&self) -> Arc<HashSet<String>> {
        self.watched.read().unwrap().clone()
    }

    /// Keeps the watched orgs current.
    fn follow_sessions(self: &Arc<Self>, pg: sqlx::PgPool) {
        let live = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(WATCHED_EVERY);
            loop {
                tick.tick().await;
                match crate::live_feed::watched(&pg).await {
                    Ok(orgs) => *live.watched.write().unwrap() = Arc::new(orgs),
                    Err(e) => tracing::warn!(error = %e, "reading live tail sessions failed"),
                }
            }
        });
    }

    fn publish(self: &Arc<Self>, by_org: HashMap<String, Vec<Vec<u8>>>) {
        let Ok(permit) = self.in_flight.clone().try_acquire_owned() else {
            tracing::debug!("live tail is behind: a batch is dropped");
            return;
        };
        let live = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(e) = live.produce(by_org).await {
                tracing::debug!(error = %e, "publishing live tail failed");
            }
        });
    }

    async fn produce(&self, by_org: HashMap<String, Vec<Vec<u8>>>) -> Result<(), String> {
        let timestamp = chrono::Utc::now();
        let records = by_org
            .into_iter()
            .flat_map(|(org, logs)| logs.into_iter().map(move |log| (org.clone(), Record { key: Some(org.as_bytes().to_vec()), value: Some(log), headers: Default::default(), timestamp })))
            .collect();
        self.live.produce(records).await
    }
}

/// Reads one fetch from `offset` and stores its logs; returns the next offset (unchanged when nothing came).
async fn drain(st: &AppState, ix: &Arc<Indexer>, out: &Arc<Outputs>, pc: &PartitionClient, topic: Topic, offset: i64) -> Result<i64, String> {
    let (mut records, mut size, mut next) = (Vec::new(), 0usize, offset);
    let started = std::time::Instant::now();
    while size < BATCH_BYTES && (records.is_empty() || started.elapsed() < BATCH_WAIT) {
        let fetched = match pc.fetch_records(next, 1..FETCH_BYTES, FETCH_WAIT_MS).await {
            Ok((fetched, _high_watermark)) => fetched,
            Err(_) if !records.is_empty() => break,
            Err(e) => {
                // Records past retention: start again from the earliest kept.
                let earliest = pc.get_offset(OffsetAt::Earliest).await.map_err(|e| e.to_string())?;
                if next < earliest {
                    tracing::warn!(offset = next, earliest, "log records expired before they were read");
                    return Ok(earliest);
                }
                return Err(e.to_string());
            }
        };
        // A compressed batch may start before the offset asked for.
        let fresh: Vec<_> = fetched.into_iter().filter(|r| r.offset >= next).collect();
        let Some(last) = fresh.last().map(|r| r.offset) else {
            if records.is_empty() {
                return Ok(offset);
            }
            break;
        };
        next = last + 1;
        size += fresh.iter().map(|r| r.record.value.as_ref().map_or(0, Vec::len)).sum::<usize>();
        records.extend(fresh.into_iter().map(|r| r.record));
    }
    let _slot = STORE_SLOTS.acquire().await.map_err(|e| e.to_string())?;
    let (rows, signals) = tokio::task::spawn_blocking(move || rows_of(topic, records)).await.map_err(|e| e.to_string())?;
    if !signals.is_empty() {
        out.signals.produce(signals).await.map_err(|e| format!("passing on other signals: {e}"))?;
    }
    store(st, ix, out, rows).await?;
    Ok(next)
}

/// Reads a partition while this server holds it.
async fn read_partition(st: AppState, ix: Arc<Indexer>, out: Arc<Outputs>, leases: Arc<Leases>, topic: (Topic, String), partition: i32, offset: Option<i64>) {
    let pc = loop {
        match out.client.partition_client(topic.1.clone(), partition, UnknownTopicHandling::Retry).await {
            Ok(pc) => break pc,
            Err(e) => {
                tracing::warn!(topic = %topic.1, partition, error = %e, "connecting to a log partition failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };
    let mut offset = match offset {
        Some(o) => o,
        None => pc.get_offset(OffsetAt::Earliest).await.unwrap_or(0),
    };
    let mut backoff = Duration::from_millis(500);
    loop {
        match drain(&st, &ix, &out, &pc, topic.0, offset).await {
            Ok(next) if next == offset => {}
            Ok(next) => {
                match leases.commit(&st.postgres, &topic.1, partition, next).await {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        tracing::warn!(topic = %topic.1, partition, error = %e, "committing a log offset failed");
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                }
                offset = next;
                backoff = Duration::from_millis(500);
            }
            Err(e) => {
                tracing::warn!(topic = %topic.1, partition, offset, error = %e, "ingesting logs failed; retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

/// Starts the ingester: leases are synced every few seconds and each held partition has a reader.
pub async fn spawn(st: AppState, ix: Arc<Indexer>) -> anyhow::Result<()> {
    let brokers = st.config.kafka_brokers.clone().ok_or_else(|| anyhow::anyhow!("OC_LOG_INGEST=kafka needs OC_KAFKA_BROKERS"))?;
    let client = Arc::new(ClientBuilder::new(brokers.split(',').map(|b| b.trim().to_string()).collect()).client_id("observecraft-log-ingester").build().await?);
    let out = Arc::new(Outputs::new(client.clone()));
    out.follow_sessions(st.postgres.clone());
    let leases = Arc::new(Leases::new());
    tokio::spawn(async move {
        let mut readers: HashMap<(String, i32), JoinHandle<()>> = HashMap::new();
        let mut tick = tokio::time::interval(SYNC_EVERY);
        loop {
            tick.tick().await;
            let partitions = match client.list_topics().await {
                Ok(topics) => topics
                    .into_iter()
                    .filter(|t| Topic::ALL.iter().any(|(_, name)| *name == t.name))
                    .flat_map(|t| t.partitions.into_iter().map(move |p| (t.name.clone(), p)))
                    .collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!(error = %e, "listing the log topics failed");
                    continue;
                }
            };
            let held = match leases.sync(&st.postgres, &partitions).await {
                Ok(held) => held,
                Err(e) => {
                    tracing::warn!(error = %e, "renewing log partition leases failed");
                    continue;
                }
            };
            let keep: std::collections::HashSet<(String, i32)> = held.iter().map(|(t, p, _)| (t.clone(), *p)).collect();
            readers.retain(|key, reader| {
                let alive = keep.contains(key) && !reader.is_finished();
                if !alive {
                    reader.abort();
                }
                alive
            });
            for (name, partition, offset) in held {
                let key = (name.clone(), partition);
                if readers.contains_key(&key) {
                    continue;
                }
                let Some((topic, _)) = Topic::ALL.into_iter().find(|(_, n)| *n == name) else { continue };
                readers.insert(key, tokio::spawn(read_partition(st.clone(), ix.clone(), out.clone(), leases.clone(), (topic, name), partition, offset)));
            }
        }
    });
    tracing::info!("log ingester reading Kafka");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_records::{any_value, AnyValue, ExportLogsServiceRequest, KeyValue, LogRecord, Resource, ResourceLogs, ScopeLogs};
    use prost::Message;

    /// Per-stage cost on benchmark-like logs: `cargo test --release -- --ignored stage_costs --nocapture`.
    #[test]
    #[ignore]
    fn stage_costs() {
        let events: Vec<_> = include_str!("log_normalize/cases.ndjson")
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .filter(|c| c["name"].as_str().unwrap().starts_with("synthetic"))
            .map(|c| c["event"].as_object().unwrap().clone())
            .collect();
        assert!(!events.is_empty());
        let n = 100_000;
        let now = chrono::Utc::now();
        let time = |label: &str, start: std::time::Instant| eprintln!("{label:<14} {:>8.2} µs/log", start.elapsed().as_secs_f64() * 1e6 / n as f64);
        let t = std::time::Instant::now();
        let rows: Vec<Row> = (0..n).filter_map(|i| {
            let mut fields = events[i % events.len()].clone();
            fields.remove("_t");
            log_normalize::normalize(crate::log_normalize::LogEvent { fields, timestamp: Some(now) }, now)
        }).collect();
        time("normalize", t);
        let t = std::time::Instant::now();
        let mut raw = Vec::new();
        for row in &rows { serde_json::to_writer(&mut raw, row).unwrap(); raw.push(b'\n'); }
        time("raw json", t);
        let orgs: Vec<String> = rows.iter().map(|r| r.org_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let ix = logproc::test_indexer(&orgs.iter().map(String::as_str).collect::<Vec<_>>());
        let (n_rows, mut rows) = (rows.len(), rows);
        let t = std::time::Instant::now();
        logproc::prepare_normalized(&ix, &mut rows, now.timestamp_millis());
        time("pipelines", t);
        let t = std::time::Instant::now();
        let (out, _, kept) = logproc::admit_for_tests(&ix, rows);
        time("admit", t);
        eprintln!("rows {n_rows} kept {kept} raw {} MiB indexed {} MiB", raw.len() >> 20, out.len() >> 20);
    }

    /// The whole path's cost on real records (`OC_TEST_KAFKA_BROKERS`, records already on `oc.otlp.logs`):
    /// `cargo test --release -- --ignored real_record_costs --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn real_record_costs() {
        let brokers = std::env::var("OC_TEST_KAFKA_BROKERS").expect("OC_TEST_KAFKA_BROKERS");
        let client = ClientBuilder::new(vec![brokers]).build().await.unwrap();
        let pc = client.partition_client("oc.otlp.logs", 0, UnknownTopicHandling::Retry).await.unwrap();
        let start = pc.get_offset(OffsetAt::Earliest).await.unwrap();
        let (fetched, _) = pc.fetch_records(start, 1..64 * 1024 * 1024, 1000).await.unwrap();
        let records: Vec<Record> = fetched.into_iter().take(40).map(|r| r.record).collect();
        let bytes: usize = records.iter().map(|r| r.value.as_ref().map_or(0, Vec::len)).sum();
        let t = std::time::Instant::now();
        let events: Vec<_> = records.iter().flat_map(|r| log_records::events(Topic::Otlp, r).unwrap()).collect();
        let n = events.len();
        eprintln!("{} records, {n} logs, {} B/log as OTLP", records.len(), bytes / n.max(1));
        let time = |label: &str, t: std::time::Instant| eprintln!("{label:<14} {:>8.2} µs/log", t.elapsed().as_secs_f64() * 1e6 / n as f64);
        time("decode", t);
        let now = chrono::Utc::now();
        let t = std::time::Instant::now();
        let rows: Vec<Row> = events.into_iter().filter_map(|e| log_normalize::normalize(e, now)).collect();
        time("normalize", t);
        let t = std::time::Instant::now();
        let mut raw = Vec::new();
        for row in &rows {
            serde_json::to_writer(&mut raw, row).unwrap();
            raw.push(b'\n');
        }
        time("raw json", t);
        let t = std::time::Instant::now();
        let compressed = zstd::bulk::compress(&raw, 3).unwrap();
        time("live zstd", t);
        let orgs: Vec<String> = rows.iter().map(|r| r.org_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let ix = logproc::test_indexer(&orgs.iter().map(String::as_str).collect::<Vec<_>>());
        let mut rows = rows;
        let t = std::time::Instant::now();
        logproc::prepare_normalized(&ix, &mut rows, now.timestamp_millis());
        time("pipelines", t);
        let t = std::time::Instant::now();
        let (out, _, _) = logproc::admit_for_tests(&ix, rows);
        time("admit", t);
        eprintln!("raw {} B/log, indexed {} B/log, live zstd {} B/log", raw.len() / n, out.len() / n, compressed.len() / n);
    }

    /// The typed path (`prepare_normalized`) indexes the same rows as the JSON one (`prepare_rows`).
    #[test]
    fn typed_pipelines_match_the_json_path() {
        let now = chrono::Utc::now();
        let rows: Vec<Row> = include_str!("log_normalize/cases.ndjson")
            .lines()
            .filter_map(|l| {
                let case: serde_json::Value = serde_json::from_str(l).unwrap();
                let mut fields = case["event"].as_object().unwrap().clone();
                fields.remove("_t");
                log_normalize::normalize(crate::log_normalize::LogEvent { fields, timestamp: Some(now) }, now)
            })
            .collect();
        let orgs: Vec<String> = rows.iter().map(|r| r.org_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let ix = logproc::test_indexer(&orgs.iter().map(String::as_str).collect::<Vec<_>>());
        // Vector's rows carry the typed attributes as `attributes`; the typed path doesn't store them.
        let vector_row = |r: &Row| {
            let mut v = serde_json::to_value(r).unwrap();
            v["attributes"] = serde_json::Value::String(serde_json::to_string(&r.attrs).unwrap());
            v
        };
        let mut json_path = logproc::prepare_rows(&ix, rows.iter().map(vector_row).collect(), now.timestamp_millis());
        let remapped = json_path.iter().zip(&rows).filter(|(a, r)| **a != vector_row(r)).count();
        for row in &mut json_path {
            row.as_object_mut().unwrap().remove("attributes");
        }
        let mut typed = rows.clone();
        logproc::prepare_normalized(&ix, &mut typed, now.timestamp_millis());
        let typed: Vec<_> = typed.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
        assert!(remapped > 0, "some cases carry standard attribute aliases");
        assert_eq!(typed, json_path);
    }

    #[test]
    fn live_tail_gets_only_watched_orgs_logs_as_normalized() {
        let now = chrono::Utc::now();
        let row = |org: &str| {
            let fields = serde_json::json!({ "source_type": "kafka", "_oc_kafka_headers": { "x-oc-org": org }, "message": "hi", "user_agent": "curl" });
            log_normalize::normalize(crate::log_normalize::LogEvent { fields: fields.as_object().unwrap().clone(), timestamp: Some(now) }, now).unwrap()
        };
        let rows = vec![row("acme"), row("other"), row("acme")];
        let watched: std::collections::HashSet<String> = ["acme".to_string()].into();
        let live = live_payloads(&rows, &watched);
        assert_eq!(live.keys().collect::<Vec<_>>(), ["acme"]);
        assert_eq!(live["acme"].len(), 2);
        let first: serde_json::Value = serde_json::from_slice(&live["acme"][0]).unwrap();
        assert_eq!((first["message"].as_str(), first["attr"]["user_agent"].as_str()), (Some("hi"), Some("curl")));
        assert!(live_payloads(&rows, &Default::default()).is_empty());
    }

    /// Rows admitted as `Row` write the same indexed lines as the same rows admitted as JSON.
    #[test]
    fn typed_admission_matches_json_admission() {
        let now = chrono::Utc::now();
        let mut rows: Vec<Row> = include_str!("log_normalize/cases.ndjson")
            .lines()
            .filter_map(|l| {
                let case: serde_json::Value = serde_json::from_str(l).unwrap();
                let mut fields = case["event"].as_object().unwrap().clone();
                fields.remove("_t");
                log_normalize::normalize(crate::log_normalize::LogEvent { fields, timestamp: Some(now) }, now)
            })
            .collect();
        let orgs: Vec<String> = rows.iter().map(|r| r.org_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let ix = logproc::test_indexer(&orgs.iter().map(String::as_str).collect::<Vec<_>>());
        logproc::prepare_normalized(&ix, &mut rows, now.timestamp_millis());
        let values: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
        let lines = |out: Vec<u8>| out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice::<serde_json::Value>(l).unwrap()).collect::<Vec<_>>();
        let (json_out, _, json_kept) = logproc::admit_for_tests(&ix, values);
        let (typed_out, _, typed_kept) = logproc::admit_for_tests(&ix, rows);
        assert_eq!(typed_kept, json_kept);
        assert!(typed_kept > 100);
        assert_eq!(lines(typed_out), lines(json_out));
    }

    #[test]
    fn partitions_are_shared_evenly_rounding_up() {
        assert_eq!(fair_share(36, 1), 36);
        assert_eq!(fair_share(36, 2), 18);
        assert_eq!(fair_share(36, 5), 8);
        assert_eq!(fair_share(3, 0), 3);
    }

    fn topic_partitions(topic: &str, n: i32) -> Vec<(String, i32)> {
        (0..n).map(|p| (topic.to_string(), p)).collect()
    }

    #[tokio::test]
    async fn servers_split_the_partitions_and_only_a_holder_commits() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let pg = &states[0].postgres;
        let topic = format!("test-logs-{:x}", rand::random::<u64>());
        let partitions = topic_partitions(&topic, 12);
        let (first, second) = (Leases::new(), Leases::new());
        assert_eq!(first.sync(pg, &partitions).await.unwrap().len(), 12, "alone, a server holds every partition");
        assert!(first.commit(pg, &topic, 3, 42).await.unwrap());
        // A second server: the first hands back half on its next sync, the second takes them.
        assert_eq!(second.sync(pg, &partitions).await.unwrap().len(), 0, "nothing is free yet");
        assert_eq!(first.sync(pg, &partitions).await.unwrap().len(), 6);
        let taken = second.sync(pg, &partitions).await.unwrap();
        assert_eq!(taken.len(), 6);
        let held_by_first: Vec<i32> = first.sync(pg, &partitions).await.unwrap().iter().map(|(_, p, _)| *p).collect();
        assert_eq!(held_by_first, (0..6).collect::<Vec<_>>());
        assert!(!first.commit(pg, &topic, 7, 1).await.unwrap(), "a partition handed over can't be committed by its old holder");
        assert!(taken.iter().all(|(_, p, _)| *p >= 6));
        // An offset survives the hand-over.
        sqlx::query("UPDATE log_ingest_partitions SET owner = NULL, lease_until = 'epoch' WHERE topic = $1").bind(&topic).execute(pg).await.unwrap();
        let resumed = second.sync(pg, &partitions).await.unwrap();
        assert_eq!(resumed.iter().find(|(_, p, _)| *p == 3).map(|(_, _, o)| *o), Some(Some(42)));
    }

    /// Against a live Kafka with the log topics (`OC_TEST_KAFKA_BROKERS`, e.g. localhost:19092) and the test databases.
    #[tokio::test]
    async fn a_partitions_logs_are_stored_raw_and_indexed() {
        let Ok(brokers) = std::env::var("OC_TEST_KAFKA_BROKERS") else { return };
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "kafka-ingest").await;
        let ix = logproc::test_indexer(&[&org]);
        let client = Arc::new(ClientBuilder::new(vec![brokers]).build().await.unwrap());
        let live = Arc::new(Outputs::new(client.clone()));
        let pc = client.partition_client("oc.otlp.logs", 0, UnknownTopicHandling::Retry).await.unwrap();
        let kv = |k: &str, v: &str| KeyValue { key: k.into(), value: Some(AnyValue { value: Some(any_value::Value::StringValue(v.into())) }) };
        // Stamped as the intake stamps them (`oc.ingest_id`), so every reader of the partition stores the same ids.
        let log = |msg: &str| LogRecord {
            time_unix_nano: chrono::Utc::now().timestamp_nanos_opt().unwrap() as u64,
            body: Some(AnyValue { value: Some(any_value::Value::StringValue(msg.into())) }),
            attributes: vec![kv("oc.ingest_id", &log_normalize::tests_uuid_v7())],
            ..Default::default()
        };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![
                ResourceLogs { resource: Some(Resource { attributes: vec![kv("service.name", "order-service")] }), scope_logs: vec![ScopeLogs { scope: None, log_records: vec![log("ERROR payment declined"), log("order accepted")] }] },
                // Another signal on the logs topic (a cluster collector's Kubernetes objects): passed on to the Collector.
                ResourceLogs { resource: Some(Resource { attributes: vec![kv("oc.signal", "k8s.objects")] }), scope_logs: vec![ScopeLogs { scope: None, log_records: vec![log("pod added")] }] },
            ],
        };
        let record = Record { key: Some(org.as_bytes().to_vec()), value: Some(req.encode_to_vec()), headers: [("x-oc-org".to_string(), org.as_bytes().to_vec())].into(), timestamp: chrono::Utc::now() };
        let signals_pc = client.partition_client(SIGNALS_TOPIC, crate::live_feed::partition_of(&org, client.list_topics().await.unwrap().iter().find(|t| t.name == SIGNALS_TOPIC).expect("oc.otlp.signals").partitions.len() as i32), UnknownTopicHandling::Retry).await.unwrap();
        let signals_from = signals_pc.get_offset(OffsetAt::Latest).await.unwrap();
        let offsets = pc.produce(vec![record], Compression::NoCompression).await.unwrap();
        let next = drain(st, &ix, &live, &pc, Topic::Otlp, offsets[0]).await.unwrap();
        assert!(next > offsets[0]);
        let (passed, _) = signals_pc.fetch_records(signals_from, 1..1 << 20, 1000).await.unwrap();
        let passed: Vec<_> = passed.into_iter().filter(|r| r.record.headers.get("x-oc-org").map(Vec::as_slice) == Some(org.as_bytes())).collect();
        // At least once: another server reading the partition (a running stack) passes it on too.
        assert!(!passed.is_empty(), "the k8s.objects resource reaches the Collector's topic with the org's header");
        for p in &passed {
            let req = ExportLogsServiceRequest::decode(p.record.value.as_deref().unwrap()).unwrap();
            assert_eq!(req.resource_logs.len(), 1, "only the signal's resource");
        }
        let count = |table: &'static str| {
            let org = org.clone();
            // Distinct ids: delivery is at least once (another server reading the partition stores them too).
            async move { st.clickhouse.query(&format!("SELECT uniqExact(id) FROM {table} WHERE org_id = ?")).bind(&org).fetch_one::<u64>().await.unwrap() }
        };
        assert_eq!(count("logs_read").await, 2);
        assert_eq!(count("logs_raw_dedup").await, 2);
        let statuses: Vec<String> = st.clickhouse.query("SELECT status FROM logs_read WHERE org_id = ? ORDER BY status").bind(&org).fetch_all().await.unwrap();
        assert_eq!(statuses, ["error", "info"]);
        // Attributes are stored once, flattened in `attr`.
        let stored: Vec<(String, String)> = st.clickhouse.query("SELECT attributes, toJSONString(attr) FROM logs_read WHERE org_id = ? LIMIT 1").bind(&org).fetch_all().await.unwrap();
        assert_eq!(stored[0].0, "");
    }
}
