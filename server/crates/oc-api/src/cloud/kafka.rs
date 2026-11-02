//! The cloud crawlers' Kafka producer: OTLP JSON metric records on `oc.otlp-json.metrics`, keyed by org
//! with the org's headers, acknowledged by every in-sync replica (`acks=all`), zstd-compressed. The
//! collector reads them like the intake's (docs/adr/0004-kafka-ingestion-buffer.md). An org's records
//! rotate over a few partitions from the one its name hashes to.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder};
use rskafka::record::Record;
use tokio::sync::Mutex;

use super::jobs::MetricSink;

pub const TOPIC: &str = "oc.otlp-json.metrics";
/// Consecutive partitions an org's records rotate over.
const SPREAD: u64 = 4;

pub struct KafkaSink {
    brokers: Vec<String>,
    topic: String,
    client: Mutex<Option<Arc<Client>>>,
    partitions: Mutex<HashMap<i32, Arc<PartitionClient>>>,
    count: Mutex<Option<i32>>,
    next: AtomicUsize,
}

/// FNV-1a: an org's home partition must not change between restarts.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The partition of an org's `turn`-th record.
pub fn partition_for(org: &str, turn: usize, partitions: i32) -> i32 {
    let n = partitions.max(1) as u64;
    ((fnv1a(org) % n + turn as u64 % SPREAD) % n) as i32
}

impl KafkaSink {
    /// `brokers`: comma-separated `host:port`. Connects on the first record.
    pub fn new(brokers: &str, topic: &str) -> Self {
        KafkaSink {
            brokers: brokers.split(',').map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect(),
            topic: topic.to_string(),
            client: Mutex::default(),
            partitions: Mutex::default(),
            count: Mutex::default(),
            next: AtomicUsize::new(0),
        }
    }

    async fn client(&self) -> Result<Arc<Client>, String> {
        let mut client = self.client.lock().await;
        if let Some(c) = client.as_ref() {
            return Ok(c.clone());
        }
        let built = Arc::new(ClientBuilder::new(self.brokers.clone()).client_id("observecraft-aws-crawler").build().await.map_err(|e| format!("Kafka: {e}"))?);
        *client = Some(built.clone());
        Ok(built)
    }

    async fn partition(&self, org: &str) -> Result<Arc<PartitionClient>, String> {
        let client = self.client().await?;
        let count = {
            let mut count = self.count.lock().await;
            if count.is_none() {
                let topics = client.list_topics().await.map_err(|e| format!("Kafka metadata: {e}"))?;
                *count = topics.iter().find(|t| t.name == self.topic).map(|t| t.partitions.len() as i32).filter(|n| *n > 0);
            }
            count.ok_or_else(|| format!("Kafka topic {} doesn't exist", self.topic))?
        };
        let partition = partition_for(org, self.next.fetch_add(1, Ordering::Relaxed), count);
        let mut cache = self.partitions.lock().await;
        if let Some(p) = cache.get(&partition) {
            return Ok(p.clone());
        }
        let p = Arc::new(client.partition_client(self.topic.clone(), partition, UnknownTopicHandling::Retry).await.map_err(|e| format!("Kafka partition {partition}: {e}"))?);
        cache.insert(partition, p.clone());
        Ok(p)
    }
}

#[axum::async_trait]
impl MetricSink for KafkaSink {
    async fn publish(&self, org: &str, headers: &[(String, String)], body: Vec<u8>) -> Result<(), String> {
        let partition = self.partition(org).await?;
        let headers: BTreeMap<String, Vec<u8>> = headers.iter().map(|(k, v)| (k.clone(), v.as_bytes().to_vec())).collect();
        let record = Record { key: Some(org.as_bytes().to_vec()), value: Some(body), headers, timestamp: chrono::Utc::now() };
        partition.produce(vec![record], Compression::Zstd).await.map(|_| ()).map_err(|e| format!("Kafka produce: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rskafka::client::partition::OffsetAt;

    #[test]
    fn orgs_rotate_over_a_few_stable_partitions() {
        let parts: std::collections::BTreeSet<i32> = (0..100).map(|t| partition_for("acme", t, 12)).collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(partition_for("acme", 0, 12), partition_for("acme", 4, 12));
        assert_eq!(partition_for("acme", 0, 1), 0);
    }

    /// Against a live Kafka with the `oc.otlp-json.metrics` topic (`OC_TEST_KAFKA_BROKERS`, e.g. localhost:19092).
    #[tokio::test]
    async fn records_carry_the_org_key_and_headers() {
        let Ok(brokers) = std::env::var("OC_TEST_KAFKA_BROKERS") else { return };
        let sink = KafkaSink::new(&brokers, TOPIC);
        let org = crate::test_support::org("aws-kafka");
        let body = br#"{"resourceMetrics":[]}"#.to_vec();
        sink.publish(&org, &[("x-oc-org".into(), org.clone()), ("x-oc-metrics-retention-days".into(), "30".into())], body.clone()).await.unwrap();

        let partition = sink.partition(&org).await.unwrap();
        // The record just written is the last one of one of the org's partitions.
        let mut found = false;
        for p in 0..sink.count.lock().await.unwrap() {
            let client = sink.client().await.unwrap().partition_client(TOPIC, p, UnknownTopicHandling::Retry).await.unwrap();
            let end = client.get_offset(OffsetAt::Latest).await.unwrap();
            if end == 0 {
                continue;
            }
            let (records, _) = client.fetch_records(end - 1, 1..1_000_000, 1_000).await.unwrap();
            if let Some(r) = records.iter().find(|r| r.record.key.as_deref() == Some(org.as_bytes())) {
                assert_eq!(r.record.value.as_deref(), Some(body.as_slice()));
                assert_eq!(r.record.headers["x-oc-org"], org.as_bytes());
                assert_eq!(r.record.headers["x-oc-metrics-retention-days"], b"30");
                found = true;
            }
        }
        assert!(found);
        drop(partition);
    }
}
