//! The Kafka log ingester (`OC_LOG_INGEST=kafka`): the server reads the log topics itself, normalizes each log
//! (`log_normalize`, as Vector's `normalize.vrl` did), writes the 72-hour copy (`oc.logs_raw_dedup`), runs the
//! indexer (`logproc::index`: pipelines, indexes, quotas, generated metrics, forwarding) and publishes live tail
//! (`oc.logs.live`). The Collector and Vector are then off the log path.
//!
//! Servers share the partitions through Postgres (`log_ingest_partitions`): each holds a lease on its fair
//! share, renewed every few seconds, and moves a partition's offset only after its logs are stored, so a
//! partition another server takes over (a restart, a scale-out) is read again from there. Delivery is
//! at least once; stored rows are deduplicated by id (`oc.logs_dedup` is a ReplacingMergeTree).

use std::collections::HashMap;
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
/// `OC_LOG_INGEST_RAW=false` skips the 72-hour copy (`oc.logs_raw_dedup`; replays and rehydration read it).
static RAW_COPY: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("OC_LOG_INGEST_RAW").map_or(true, |v| v != "false"));
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

/// The rows of a fetch's records: decoding and normalization, off the async workers.
fn rows_of(topic: Topic, records: Vec<Record>) -> Vec<Row> {
    let now = chrono::Utc::now();
    let mut rows = Vec::new();
    for record in &records {
        match log_records::events(topic, record) {
            Ok(events) => rows.extend(events.into_iter().filter_map(|e| log_normalize::normalize(e, now))),
            Err(e) => tracing::warn!(error = %e, "skipping an undecodable log record"),
        }
    }
    rows
}

/// Stores rows: the 72-hour copy and the indexed rows together, then live tail (best effort).
async fn store(st: &AppState, ix: &Arc<Indexer>, live: &Arc<Live>, rows: Vec<Row>) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp_millis();
    let ix_rows = ix.clone();
    let (raw, values, by_org) = tokio::task::spawn_blocking(move || {
        let mut rows = rows;
        // The 72-hour copy and live tail get the logs as normalized, before the org's pipelines (as from Vector).
        let mut raw = Vec::with_capacity(rows.len() * 2048);
        let mut by_org: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
        for row in &rows {
            let start = raw.len();
            if serde_json::to_writer(&mut raw, row).is_ok() {
                by_org.entry(row.org_id.clone()).or_default().push(raw[start..].to_vec());
                raw.push(b'\n');
            }
        }
        logproc::prepare_normalized(&ix_rows, &mut rows, now);
        let values: Vec<_> = rows.iter().filter_map(|row| serde_json::to_value(row).ok()).collect();
        (raw, values, by_org)
    })
    .await
    .map_err(|e| e.to_string())?;
    let prepared = values;
    let raw_copy = async {
        if *RAW_COPY {
            logproc::insert_rows(st, "oc.logs_raw_dedup", raw).await
        } else {
            Ok(())
        }
    };
    let (raw_stored, indexed) = tokio::join!(raw_copy, logproc::index(st, ix, prepared));
    raw_stored.map_err(|e| format!("storing raw logs: {e}"))?;
    indexed.map_err(|e| format!("indexing logs: {e}"))?;
    live.publish(by_org);
    Ok(())
}

/// Live tail's producer: each org's logs on its `oc.logs.live` partition, as Vector's `live` sink keyed them.
struct Live {
    client: Arc<Client>,
    in_flight: Arc<tokio::sync::Semaphore>,
    partitions: tokio::sync::OnceCell<i32>,
    clients: tokio::sync::Mutex<HashMap<i32, Arc<PartitionClient>>>,
}

impl Live {
    fn new(client: Arc<Client>) -> Self {
        Live { client, in_flight: Arc::new(tokio::sync::Semaphore::new(LIVE_IN_FLIGHT)), partitions: Default::default(), clients: Default::default() }
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
        let count = *self
            .partitions
            .get_or_try_init(|| async {
                let topics = self.client.list_topics().await.map_err(|e| e.to_string())?;
                topics.iter().find(|t| t.name == crate::live_feed::TOPIC).map(|t| t.partitions.len() as i32).ok_or_else(|| "no live tail topic".to_string())
            })
            .await?;
        let mut per_partition: HashMap<i32, Vec<Record>> = HashMap::new();
        let timestamp = chrono::Utc::now();
        for (org, logs) in by_org {
            let records = per_partition.entry(crate::live_feed::partition_of(&org, count)).or_default();
            records.extend(logs.into_iter().map(|log| Record { key: Some(org.as_bytes().to_vec()), value: Some(log), headers: Default::default(), timestamp }));
        }
        for (partition, records) in per_partition {
            let pc = {
                let mut clients = self.clients.lock().await;
                match clients.get(&partition) {
                    Some(pc) => pc.clone(),
                    None => {
                        let pc = Arc::new(self.client.partition_client(crate::live_feed::TOPIC, partition, UnknownTopicHandling::Retry).await.map_err(|e| e.to_string())?);
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

/// Reads one fetch from `offset` and stores its logs; returns the next offset (unchanged when nothing came).
async fn drain(st: &AppState, ix: &Arc<Indexer>, live: &Arc<Live>, pc: &PartitionClient, topic: Topic, offset: i64) -> Result<i64, String> {
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
    let rows = tokio::task::spawn_blocking(move || rows_of(topic, records)).await.map_err(|e| e.to_string())?;
    store(st, ix, live, rows).await?;
    Ok(next)
}

/// Reads a partition while this server holds it.
async fn read_partition(st: AppState, ix: Arc<Indexer>, live: Arc<Live>, leases: Arc<Leases>, topic: (Topic, String), partition: i32, offset: Option<i64>) {
    let pc = loop {
        match live.client.partition_client(topic.1.clone(), partition, UnknownTopicHandling::Retry).await {
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
        match drain(&st, &ix, &live, &pc, topic.0, offset).await {
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
    let live = Arc::new(Live::new(client.clone()));
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
                readers.insert(key, tokio::spawn(read_partition(st.clone(), ix.clone(), live.clone(), leases.clone(), (topic, name), partition, offset)));
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
        let t = std::time::Instant::now();
        let values: Vec<_> = rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
        time("to_value", t);
        let orgs: Vec<String> = rows.iter().map(|r| r.org_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let ix = logproc::test_indexer(&orgs.iter().map(String::as_str).collect::<Vec<_>>());
        let t = std::time::Instant::now();
        let prepared = logproc::prepare_rows(&ix, values, now.timestamp_millis());
        time("pipelines", t);
        let t = std::time::Instant::now();
        let (out, _, kept) = logproc::admit_for_tests(&ix, prepared);
        time("admit", t);
        eprintln!("rows {} kept {kept} raw {} MiB indexed {} MiB", rows.len(), raw.len() >> 20, out.len() >> 20);
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
        let prepared: Vec<_> = rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
        time("to_value", t);
        let t = std::time::Instant::now();
        let (out, _, _) = logproc::admit_for_tests(&ix, prepared);
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
        let json_path = logproc::prepare_rows(&ix, rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect(), now.timestamp_millis());
        let mut typed = rows.clone();
        logproc::prepare_normalized(&ix, &mut typed, now.timestamp_millis());
        let typed: Vec<_> = typed.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
        let remapped = json_path.iter().zip(&rows).filter(|(a, r)| **a != serde_json::to_value(r).unwrap()).count();
        assert!(remapped > 0, "some cases carry standard attribute aliases");
        assert_eq!(typed, json_path);
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
        let live = Arc::new(Live::new(client.clone()));
        let pc = client.partition_client("oc.otlp.logs", 0, UnknownTopicHandling::Retry).await.unwrap();
        let kv = |k: &str, v: &str| KeyValue { key: k.into(), value: Some(AnyValue { value: Some(any_value::Value::StringValue(v.into())) }) };
        let log = |msg: &str| LogRecord { time_unix_nano: chrono::Utc::now().timestamp_nanos_opt().unwrap() as u64, body: Some(AnyValue { value: Some(any_value::Value::StringValue(msg.into())) }), ..Default::default() };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs { resource: Some(Resource { attributes: vec![kv("service.name", "order-service")] }), scope_logs: vec![ScopeLogs { scope: None, log_records: vec![log("ERROR payment declined"), log("order accepted")] }] }],
        };
        let record = Record { key: Some(org.as_bytes().to_vec()), value: Some(req.encode_to_vec()), headers: [("x-oc-org".to_string(), org.as_bytes().to_vec())].into(), timestamp: chrono::Utc::now() };
        let offsets = pc.produce(vec![record], Compression::NoCompression).await.unwrap();
        let next = drain(st, &ix, &live, &pc, Topic::Otlp, offsets[0]).await.unwrap();
        assert!(next > offsets[0]);
        let count = |table: &'static str| {
            let org = org.clone();
            async move { st.clickhouse.query(&format!("SELECT count() FROM {table} WHERE org_id = ?")).bind(&org).fetch_one::<u64>().await.unwrap() }
        };
        assert_eq!(count("logs_read").await, 2);
        assert_eq!(count("logs_raw_dedup").await, 2);
        let statuses: Vec<String> = st.clickhouse.query("SELECT status FROM logs_read WHERE org_id = ? ORDER BY status").bind(&org).fetch_all().await.unwrap();
        assert_eq!(statuses, ["error", "info"]);
    }
}
