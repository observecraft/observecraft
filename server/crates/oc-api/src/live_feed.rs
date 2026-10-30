//! Live tail's source: the Kafka topic `oc.logs.live`, where the Vector aggregator writes every
//! normalized log keyed by organization (docs/adr/0004-kafka-ingestion-buffer.md).
//!
//! Vector partitions by `murmur2(org)` like Kafka's Java clients, so an org's logs are all on one
//! partition and a live tail session needs only that one. Each partition some session needs has one
//! reader on this server, started at the partition's latest offset when its first session subscribes
//! and stopped when its last one leaves; it hands each record to the sessions of the record's org.
//! Nothing is committed: live tail shows what arrives while it's open, as before with NATS.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rskafka::client::partition::{OffsetAt, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder};
use tokio::sync::broadcast;

pub const TOPIC: &str = "oc.logs.live";
/// Records a slow session may lag behind before it skips ahead.
const SESSION_BUFFER: usize = 4096;

type Sessions = Arc<Mutex<HashMap<String, broadcast::Sender<Arc<Vec<u8>>>>>>;

struct Reader {
    orgs: Sessions,
    task: tokio::task::JoinHandle<()>,
}

pub struct LiveFeed {
    brokers: Vec<String>,
    client: tokio::sync::Mutex<Option<Arc<Client>>>,
    readers: Mutex<HashMap<i32, Reader>>,
}

/// One live tail session's records (JSON, as Vector wrote them). Leaving drops the subscription.
pub struct Subscription {
    pub rx: broadcast::Receiver<Arc<Vec<u8>>>,
    feed: Arc<LiveFeed>,
    org: String,
    partition: i32,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.feed.release(&self.org, self.partition, self.rx.len());
    }
}

impl LiveFeed {
    /// `brokers`: comma-separated `host:port`. Connects lazily, on the first session.
    pub fn new(brokers: &str) -> Self {
        LiveFeed { brokers: brokers.split(',').map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect(), client: Default::default(), readers: Mutex::default() }
    }

    async fn client(&self) -> Result<Arc<Client>, String> {
        let mut c = self.client.lock().await;
        if let Some(c) = c.as_ref() {
            return Ok(c.clone());
        }
        let built = Arc::new(ClientBuilder::new(self.brokers.clone()).client_id("observecraft-server").build().await.map_err(|e| format!("Kafka: {e}"))?);
        *c = Some(built.clone());
        Ok(built)
    }

    /// Whether Kafka answers (readiness, informational).
    pub async fn reachable(&self) -> bool {
        match self.client().await {
            Ok(c) => tokio::time::timeout(Duration::from_secs(2), c.list_topics()).await.is_ok_and(|r| r.is_ok()),
            Err(_) => false,
        }
    }

    pub async fn subscribe(self: &Arc<Self>, org: &str) -> Result<Subscription, String> {
        let client = self.client().await?;
        let topics = client.list_topics().await.map_err(|e| format!("Kafka metadata: {e}"))?;
        let partitions = topics.iter().find(|t| t.name == TOPIC).map(|t| t.partitions.len() as i32).filter(|n| *n > 0).ok_or_else(|| format!("Kafka topic {TOPIC} doesn't exist"))?;
        let partition = partition_of(org, partitions);
        let mut readers = self.readers.lock().unwrap();
        let reader = readers.entry(partition).or_insert_with(|| {
            let orgs: Sessions = Arc::default();
            let task = tokio::spawn(read(client.clone(), partition, orgs.clone()));
            Reader { orgs, task }
        });
        let rx = reader.orgs.lock().unwrap().entry(org.to_string()).or_insert_with(|| broadcast::channel(SESSION_BUFFER).0).subscribe();
        Ok(Subscription { rx, feed: self.clone(), org: org.to_string(), partition })
    }

    fn release(&self, org: &str, partition: i32, _pending: usize) {
        let mut readers = self.readers.lock().unwrap();
        let Some(reader) = readers.get(&partition) else { return };
        let empty = {
            let mut orgs = reader.orgs.lock().unwrap();
            // The dropped receiver still counts while `drop` runs: 1 means it was the last.
            if orgs.get(org).is_some_and(|tx| tx.receiver_count() <= 1) {
                orgs.remove(org);
            }
            orgs.is_empty()
        };
        if empty {
            if let Some(r) = readers.remove(&partition) {
                r.task.abort();
            }
        }
    }
}

/// Reads one partition from its latest offset, handing records to their org's sessions.
async fn read(client: Arc<Client>, partition: i32, orgs: Sessions) {
    loop {
        let pc = match client.partition_client(TOPIC, partition, UnknownTopicHandling::Retry).await {
            Ok(pc) => pc,
            Err(e) => {
                tracing::warn!(error = %e, partition, "live tail: Kafka partition unavailable");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let mut offset = match pc.get_offset(OffsetAt::Latest).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(error = %e, partition, "live tail: reading the latest offset failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        loop {
            match pc.fetch_records(offset, 1..8 * 1024 * 1024, 500).await {
                Ok((records, _high)) => {
                    for r in records {
                        offset = r.offset + 1;
                        let (Some(key), Some(value)) = (r.record.key, r.record.value) else { continue };
                        let Ok(org) = std::str::from_utf8(&key) else { continue };
                        if let Some(tx) = orgs.lock().unwrap().get(org) {
                            let _ = tx.send(Arc::new(value));
                        }
                    }
                }
                Err(e) => {
                    // Fell behind retention, or the broker moved: start again from the latest offset.
                    tracing::warn!(error = %e, partition, "live tail: fetch failed");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    break;
                }
            }
        }
    }
}

/// Kafka's default partitioner for keyed records (Java clients; librdkafka `murmur2_random`).
pub fn partition_of(key: &str, partitions: i32) -> i32 {
    ((murmur2(key.as_bytes()) & 0x7fff_ffff) % partitions.max(1) as u32) as i32
}

/// MurmurHash2 as Kafka's `Utils.murmur2` computes it (seed 0x9747b28c).
pub fn murmur2(data: &[u8]) -> u32 {
    const M: u32 = 0x5bd1_e995;
    const R: u32 = 24;
    let len = data.len();
    let mut h: u32 = 0x9747_b28c ^ len as u32;
    let chunks = len / 4;
    for i in 0..chunks {
        let mut k = u32::from_le_bytes([data[4 * i], data[4 * i + 1], data[4 * i + 2], data[4 * i + 3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M);
        h ^= k;
    }
    let tail = &data[chunks * 4..];
    match tail.len() {
        3 => {
            h ^= (tail[2] as u32) << 16;
            h ^= (tail[1] as u32) << 8;
            h ^= tail[0] as u32;
            h = h.wrapping_mul(M);
        }
        2 => {
            h ^= (tail[1] as u32) << 8;
            h ^= tail[0] as u32;
            h = h.wrapping_mul(M);
        }
        1 => {
            h ^= tail[0] as u32;
            h = h.wrapping_mul(M);
        }
        _ => {}
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur2_matches_kafkas_java_client() {
        // Values from Kafka's UtilsTest.testMurmur2 (as signed ints there).
        assert_eq!(murmur2(b"21") as i32, -973932308);
        assert_eq!(murmur2(b"foobar") as i32, -790332482);
        assert_eq!(murmur2(b"a-little-bit-long-string") as i32, -985981536);
        assert_eq!(murmur2(b"a-little-bit-longer-string") as i32, -1486304829);
        assert_eq!(murmur2(b"lkjh234lh9fiuh90y23oiuhsafujhadof229phr9h19h89h8") as i32, -58897971);
        assert_eq!(murmur2(b"abc") as i32, 479470107);
        assert!((0..12).contains(&partition_of("acme", 12)));
    }

    /// Against a live Kafka with the `oc.logs.live` topic (`OC_TEST_KAFKA_BROKERS`, e.g. localhost:19092).
    #[tokio::test]
    async fn sessions_get_their_orgs_records_only() {
        let Ok(brokers) = std::env::var("OC_TEST_KAFKA_BROKERS") else { return };
        let feed = Arc::new(LiveFeed::new(&brokers));
        let org = format!("live-test-{}", std::process::id());
        let mut sub = feed.subscribe(&org).await.expect("subscribe");
        // Let the reader read the latest offset before producing.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let client = feed.client().await.unwrap();
        let partitions = client.list_topics().await.unwrap().into_iter().find(|t| t.name == TOPIC).unwrap().partitions.len() as i32;
        let pc = client.partition_client(TOPIC, partition_of(&org, partitions), UnknownTopicHandling::Retry).await.unwrap();
        let record = |key: &str, value: &str| rskafka::record::Record { key: Some(key.as_bytes().to_vec()), value: Some(value.as_bytes().to_vec()), headers: Default::default(), timestamp: chrono::Utc::now() };
        // Another org on the same partition is not delivered.
        pc.produce(vec![record("someone-else", "{\"x\":0}"), record(&org, "{\"x\":1}")], rskafka::client::partition::Compression::NoCompression).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(10), sub.rx.recv()).await.expect("a record").unwrap();
        assert_eq!(got.as_slice(), b"{\"x\":1}");
        drop(sub);
        assert!(feed.readers.lock().unwrap().is_empty(), "the reader stops with its last session");
    }
}
