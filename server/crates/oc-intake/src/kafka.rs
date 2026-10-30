//! The intake's Kafka producer (docs/adr/0004-kafka-ingestion-buffer.md).
//!
//! Every accepted request becomes one record on its signal's topic, keyed by organization, with the
//! org's headers (`x-oc-org`, retention, …) as record headers. A request is acknowledged to its sender
//! only once Kafka acknowledged the record to all in-sync replicas (`acks=all`).
//!
//! Each partition has one batching producer: concurrent requests to a partition are written in one
//! produce call after a short linger, zstd-compressed. An organization's records spread over
//! `spread` consecutive partitions from the one its name hashes to, so one large organization isn't
//! bound to a single partition (and consumer) while each org still touches few partitions.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
use rskafka::client::producer::aggregator::RecordAggregator;
use rskafka::client::producer::{BatchProducer, BatchProducerBuilder};
use rskafka::client::{Client, ClientBuilder};
use rskafka::record::Record;
use tokio::sync::Mutex;

/// What a request carries, i.e. where it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sink {
    Traces,
    Metrics,
    Logs,
    /// Datadog / JSON log events (Vector normalizes them).
    JsonLogs,
}

impl Sink {
    /// The topic of a body: OTLP protobuf and OTLP JSON have their own topics, so consumers never sniff.
    pub fn topic(self, json: bool) -> &'static str {
        match (self, json) {
            (Sink::Traces, false) => "oc.otlp.traces",
            (Sink::Metrics, false) => "oc.otlp.metrics",
            (Sink::Logs, false) => "oc.otlp.logs",
            (Sink::Traces, true) => "oc.otlp-json.traces",
            (Sink::Metrics, true) => "oc.otlp-json.metrics",
            (Sink::Logs, true) => "oc.otlp-json.logs",
            (Sink::JsonLogs, _) => "oc.logs.json",
        }
    }
}

/// Largest batch a partition producer sends in one request (bytes, before compression).
const MAX_BATCH_BYTES: usize = 16 * 1024 * 1024;
/// How long a producer waits for more records before sending a batch.
const LINGER: Duration = Duration::from_millis(5);
/// How often partition counts are re-read (topics can gain partitions).
const METADATA_TTL: Duration = Duration::from_secs(300);

type Producer = Arc<BatchProducer<RecordAggregator>>;

pub struct Kafka {
    client: Client,
    spread: usize,
    partitions: Mutex<HashMap<String, (i32, Instant)>>,
    producers: Mutex<HashMap<(String, i32), Producer>>,
    next: AtomicUsize,
}

impl Kafka {
    /// Connects to `brokers` (comma-separated `host:port`).
    pub async fn connect(brokers: &str, spread: usize) -> anyhow::Result<Self> {
        let list: Vec<String> = brokers.split(',').map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect();
        anyhow::ensure!(!list.is_empty(), "OC_KAFKA_BROKERS is empty");
        let client = ClientBuilder::new(list).client_id("oc-intake").build().await?;
        Ok(Kafka { client, spread: spread.max(1), partitions: Mutex::default(), producers: Mutex::default(), next: AtomicUsize::new(0) })
    }

    async fn partition_count(&self, topic: &str) -> Result<i32, String> {
        if let Some((n, at)) = self.partitions.lock().await.get(topic) {
            if at.elapsed() < METADATA_TTL {
                return Ok(*n);
            }
        }
        let topics = self.client.list_topics().await.map_err(|e| format!("Kafka metadata: {e}"))?;
        let mut cache = self.partitions.lock().await;
        for t in topics {
            cache.insert(t.name.clone(), (t.partitions.len() as i32, Instant::now()));
        }
        cache.get(topic).map(|(n, _)| *n).filter(|n| *n > 0).ok_or_else(|| format!("Kafka topic {topic} doesn't exist"))
    }

    async fn producer(&self, topic: &str, partition: i32) -> Result<Producer, String> {
        let key = (topic.to_string(), partition);
        if let Some(p) = self.producers.lock().await.get(&key) {
            return Ok(p.clone());
        }
        let client: Arc<PartitionClient> = Arc::new(self.client.partition_client(topic, partition, UnknownTopicHandling::Retry).await.map_err(|e| format!("Kafka partition {topic}/{partition}: {e}"))?);
        let producer = Arc::new(BatchProducerBuilder::new(client).with_linger(LINGER).with_compression(Compression::Zstd).build(RecordAggregator::new(MAX_BATCH_BYTES)));
        Ok(self.producers.lock().await.entry(key).or_insert(producer).clone())
    }

    /// Writes one record and waits for Kafka's acknowledgement.
    pub async fn produce(&self, topic: &str, org: &str, value: Vec<u8>, headers: BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        let partitions = self.partition_count(topic).await?;
        let turn = self.next.fetch_add(1, Ordering::Relaxed);
        let partition = partition_for(org, turn, self.spread, partitions);
        let producer = self.producer(topic, partition).await?;
        let record = Record { key: Some(org.as_bytes().to_vec()), value: Some(value), headers, timestamp: chrono::Utc::now() };
        producer.produce(record).await.map(|_| ()).map_err(|e| format!("Kafka produce {topic}/{partition}: {e}"))
    }
}

/// FNV-1a: a stable hash of the organization (the partition must not change between restarts).
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The partition of an org's `turn`-th record: one of `spread` consecutive partitions from its home.
pub fn partition_for(org: &str, turn: usize, spread: usize, partitions: i32) -> i32 {
    let n = partitions.max(1) as u64;
    let home = fnv1a(org) % n;
    ((home + (turn % spread.max(1)) as u64) % n) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orgs_spread_over_a_stable_set_of_partitions() {
        let parts: std::collections::BTreeSet<i32> = (0..100).map(|t| partition_for("acme", t, 4, 12)).collect();
        assert_eq!(parts.len(), 4);
        // Stable across calls (and restarts: no random seed).
        assert_eq!(partition_for("acme", 0, 4, 12), partition_for("acme", 4, 4, 12));
        // Wraps around the last partition, and a spread larger than the topic uses every partition.
        assert!((0..100).all(|t| (0..12).contains(&partition_for("acme", t, 4, 12))));
        assert_eq!((0..100).map(|t| partition_for("x", t, 50, 3)).collect::<std::collections::BTreeSet<_>>().len(), 3);
    }

    #[test]
    fn topics_by_signal_and_encoding() {
        assert_eq!(Sink::Traces.topic(false), "oc.otlp.traces");
        assert_eq!(Sink::Metrics.topic(true), "oc.otlp-json.metrics");
        assert_eq!(Sink::JsonLogs.topic(false), "oc.logs.json");
    }
}
