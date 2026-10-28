//! Data Streams Monitoring → Kafka Console: clusters, topics, brokers, consumer groups and
//! connectors, from the metrics Kafka already reports. Both naming schemes are read:
//! - the Datadog Agent's `kafka` check (broker JMX: `kafka.messages_in.rate`, `kafka.net.bytes_in.rate`,
//!   `kafka.replication.*`, `kafka.topic.*.rate`, per broker host) and its `kafka_consumer` check
//!   (`kafka.consumer_lag`, `kafka.consumer_offset`, `kafka.broker_offset`, `kafka.estimated_consumer_lag`
//!   tagged `topic`, `partition`, `consumer_group`, `kafka_cluster_id`, plus the cluster metadata it
//!   collects: `kafka.broker.*`, `kafka.topic.partitions`, `kafka.partition.*`, `kafka.consumer_group.members`,
//!   `kafka.connector.*`). Names from `integrations-core/kafka/metadata.csv` and
//!   `integrations-core/kafka_consumer/metadata.csv` (BSD-3-Clause, Copyright (c) Datadog, Inc.);
//! - Kafka Connect workers through the `confluent_platform` check
//!   (`confluent.kafka.connect.worker.connector_*_task_count`, tagged `connector`,
//!   `integrations-core/confluent_platform/metadata.csv`);
//! - the OpenTelemetry Collector's `kafkametrics` receiver (`kafka.brokers`, `kafka.topic.partitions`,
//!   `kafka.partition.current_offset`, `kafka.partition.replicas[_in_sync]`, `kafka.consumer_group.lag`,
//!   `kafka.consumer_group.offset`, `kafka.consumer_group.members`, attributes `topic`, `partition`, `group`).
//!
//! Every series is read per chart interval; current values are the last interval's, rates the
//! average over the range. A cluster is the `kafka_cluster` / `cluster_name` / `kafka.cluster.alias`
//! / `kafka_cluster_id` tag; series without one belong to the only cluster that has a name, else to `default`.
//!
//! Tenancy: metrics are read through `series_values` (`OrgId = {oc_org:String}`), monitors with
//! `org_id = $1`, events with `OrgId = {oc_org:String}`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::State;
use axum::Json;
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::metric_inventory::{chart_step, series_values, SeriesValue};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// Metric names (see the module docs for their sources).
pub(crate) mod names {
    // Datadog `kafka` check (broker JMX).
    pub const MESSAGES_IN: &str = "kafka.messages_in.rate";
    pub const BYTES_IN: &str = "kafka.net.bytes_in.rate";
    pub const BYTES_OUT: &str = "kafka.net.bytes_out.rate";
    pub const UNDER_REPLICATED_JMX: &str = "kafka.replication.under_replicated_partitions";
    pub const OFFLINE_JMX: &str = "kafka.replication.offline_partitions_count";
    pub const ACTIVE_CONTROLLER: &str = "kafka.replication.active_controller_count";
    pub const BROKER_PARTITIONS_JMX: &str = "kafka.replication.partition_count";
    pub const BROKER_LEADERS_JMX: &str = "kafka.replication.leader_count";
    pub const TOPIC_MESSAGES_IN: &str = "kafka.topic.messages_in.rate";
    pub const TOPIC_BYTES_IN: &str = "kafka.topic.net.bytes_in.rate";
    pub const TOPIC_BYTES_OUT: &str = "kafka.topic.net.bytes_out.rate";
    // Datadog `kafka_consumer` check.
    pub const CONSUMER_LAG: &str = "kafka.consumer_lag";
    pub const CONSUMER_OFFSET: &str = "kafka.consumer_offset";
    pub const BROKER_OFFSET: &str = "kafka.broker_offset";
    pub const LAG_SECONDS: &str = "kafka.estimated_consumer_lag";
    pub const BROKER_COUNT: &str = "kafka.broker.count";
    pub const BROKER_PARTITIONS: &str = "kafka.broker.partition_count";
    pub const BROKER_LEADERS: &str = "kafka.broker.leader_count";
    pub const CONTROLLER_ID: &str = "kafka.cluster.controller_id";
    pub const TOPIC_MESSAGE_RATE: &str = "kafka.topic.message_rate";
    pub const TOPIC_SIZE: &str = "kafka.topic.size";
    pub const PARTITION_UNDER_REPLICATED: &str = "kafka.partition.under_replicated";
    pub const PARTITION_OFFLINE: &str = "kafka.partition.offline";
    pub const CONNECTOR_TASK_COUNT: &str = "kafka.connector.task.count";
    pub const CONNECTOR_TASKS: &str = "kafka.connector.tasks";
    // Both the `kafka_consumer` check and the OpenTelemetry receiver.
    pub const TOPIC_PARTITIONS: &str = "kafka.topic.partitions";
    pub const PARTITION_REPLICAS: &str = "kafka.partition.replicas";
    pub const GROUP_MEMBERS: &str = "kafka.consumer_group.members";
    // OpenTelemetry `kafkametrics` receiver.
    pub const OTEL_BROKERS: &str = "kafka.brokers";
    pub const OTEL_PARTITION_OFFSET: &str = "kafka.partition.current_offset";
    pub const OTEL_IN_SYNC: &str = "kafka.partition.replicas_in_sync";
    pub const OTEL_GROUP_OFFSET: &str = "kafka.consumer_group.offset";
    pub const OTEL_GROUP_LAG: &str = "kafka.consumer_group.lag";
    // Confluent Platform check (Kafka Connect worker JMX).
    pub const CONNECT_TOTAL: &str = "confluent.kafka.connect.worker.connector_total_task_count";
    pub const CONNECT_RUNNING: &str = "confluent.kafka.connect.worker.connector_running_task_count";
    pub const CONNECT_FAILED: &str = "confluent.kafka.connect.worker.connector_failed_task_count";
}
use names::*;

const ALL_METRICS: &[&str] = &[
    MESSAGES_IN,
    BYTES_IN,
    BYTES_OUT,
    UNDER_REPLICATED_JMX,
    OFFLINE_JMX,
    ACTIVE_CONTROLLER,
    BROKER_PARTITIONS_JMX,
    BROKER_LEADERS_JMX,
    TOPIC_MESSAGES_IN,
    TOPIC_BYTES_IN,
    TOPIC_BYTES_OUT,
    CONSUMER_LAG,
    CONSUMER_OFFSET,
    BROKER_OFFSET,
    LAG_SECONDS,
    BROKER_COUNT,
    BROKER_PARTITIONS,
    BROKER_LEADERS,
    CONTROLLER_ID,
    TOPIC_MESSAGE_RATE,
    TOPIC_SIZE,
    PARTITION_UNDER_REPLICATED,
    PARTITION_OFFLINE,
    CONNECTOR_TASK_COUNT,
    CONNECTOR_TASKS,
    TOPIC_PARTITIONS,
    PARTITION_REPLICAS,
    GROUP_MEMBERS,
    OTEL_BROKERS,
    OTEL_PARTITION_OFFSET,
    OTEL_IN_SYNC,
    OTEL_GROUP_OFFSET,
    OTEL_GROUP_LAG,
    CONNECT_TOTAL,
    CONNECT_RUNNING,
    CONNECT_FAILED,
];

/// The metrics behind consumer lag (what the Data Streams map reads).
pub(crate) const LAG_METRICS: &[&str] = &[CONSUMER_LAG, OTEL_GROUP_LAG, LAG_SECONDS];

/// Consumer lag above these is high: the map turns the edge red and the health check fails.
pub(crate) const LAG_HIGH_SECONDS: f64 = 60.0;
pub(crate) const LAG_HIGH_MESSAGES: f64 = 10_000.0;

/// High consumer lag: its time when known (the Agent's estimate), else its message count.
pub(crate) fn lag_is_high(messages: Option<f64>, seconds: Option<f64>) -> bool {
    match seconds {
        Some(s) => s > LAG_HIGH_SECONDS,
        None => messages.is_some_and(|m| m > LAG_HIGH_MESSAGES),
    }
}

const CLUSTER_KEYS: &[&str] = &["kafka_cluster", "cluster_name", "kafka.cluster.alias", "kafka_cluster_id"];
const GROUP_KEYS: &[&str] = &["consumer_group", "group"];
const DEFAULT_CLUSTER: &str = "default";

/// One series over the range: its labels and points per interval, oldest first.
#[derive(Debug)]
pub(crate) struct Series {
    pub label: SeriesValue,
    pub points: Vec<(i64, f64)>,
}

impl Series {
    pub fn metric(&self) -> &str {
        &self.label.metric
    }
    pub fn tag(&self, keys: &[&str]) -> Option<String> {
        self.label.tag(keys)
    }
    pub fn latest(&self) -> Option<f64> {
        self.points.last().map(|p| p.1)
    }
    pub fn mean(&self) -> Option<f64> {
        (!self.points.is_empty()).then(|| self.points.iter().map(|p| p.1).sum::<f64>() / self.points.len() as f64)
    }
    /// Per-second growth of a counter-like gauge (an offset) from interval to interval, at the later one.
    pub fn growth(&self) -> Vec<(i64, f64)> {
        self.points.windows(2).filter(|w| w[1].0 > w[0].0 && w[1].1 >= w[0].1).map(|w| (w[1].0, (w[1].1 - w[0].1) * 1000.0 / (w[1].0 - w[0].0) as f64)).collect()
    }
    /// Average per-second growth over the range.
    pub fn growth_rate(&self) -> Option<f64> {
        let g = self.growth();
        (!g.is_empty()).then(|| g.iter().map(|p| p.1).sum::<f64>() / g.len() as f64)
    }
}

/// Interval values → one [`Series`] per metric and label set.
pub(crate) fn group_series(values: &[SeriesValue]) -> Vec<Series> {
    let mut by_key: BTreeMap<String, Series> = BTreeMap::new();
    for v in values {
        let mut res: Vec<_> = v.res.iter().collect();
        res.sort();
        let mut attrs: Vec<_> = v.attrs.iter().collect();
        attrs.sort();
        let key = format!("{}|{res:?}|{attrs:?}", v.metric);
        by_key.entry(key).or_insert_with(|| Series { label: v.clone(), points: Vec::new() }).points.push((v.t, v.value));
    }
    let mut out: Vec<Series> = by_key.into_values().collect();
    for s in &mut out {
        s.points.sort_by_key(|p| p.0);
    }
    out
}

/// Sum of `f`'s points per time across `series`.
fn sum_by_t<'a>(series: impl Iterator<Item = &'a Series>, f: impl Fn(&Series) -> Vec<(i64, f64)>) -> Vec<(i64, f64)> {
    let mut acc: BTreeMap<i64, f64> = BTreeMap::new();
    for s in series {
        for (t, v) in f(s) {
            *acc.entry(t).or_default() += v;
        }
    }
    acc.into_iter().collect()
}

fn mean_of(points: &[(i64, f64)]) -> Option<f64> {
    (!points.is_empty()).then(|| points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64)
}

// ================================================================ response

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum KafkaCheckStatus {
    Passing,
    Failing,
    NoData,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KafkaHealthCheck {
    pub cluster: String,
    pub name: String,
    pub status: KafkaCheckStatus,
    pub detail: String,
}

#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaCluster {
    pub name: String,
    pub brokers: Option<u64>,
    pub topics: u64,
    pub partitions: Option<u64>,
    pub under_replicated: Option<f64>,
    pub offline: Option<f64>,
    /// Messages per second produced to the cluster, averaged over the range.
    pub messages_in: Option<f64>,
    pub consumer_groups: u64,
    pub connectors: u64,
    pub checks_passing: u64,
    pub checks_failing: u64,
    /// Monitors whose query names Kafka and this cluster.
    pub monitors: u64,
}

#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaTopic {
    pub cluster: String,
    pub name: String,
    pub partitions: Option<u64>,
    pub replication_factor: Option<u64>,
    pub under_replicated: Option<u64>,
    /// Messages per second produced (average).
    pub messages_in: Option<f64>,
    /// Messages per second consumed, all consumer groups together (average).
    pub messages_out: Option<f64>,
    pub bytes_in: Option<f64>,
    pub bytes_out: Option<f64>,
    /// Messages in the topic.
    pub size: Option<f64>,
    /// The largest lag of a consumer group on the topic (messages, all partitions).
    pub consumer_lag_max: Option<f64>,
    pub consumer_groups: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaBroker {
    pub cluster: String,
    /// The broker id, else its host.
    pub id: String,
    pub host: Option<String>,
    pub partitions: Option<u64>,
    pub leaders: Option<u64>,
    pub under_replicated: Option<f64>,
    pub messages_in: Option<f64>,
    pub bytes_in: Option<f64>,
    pub bytes_out: Option<f64>,
    pub controller: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KafkaGroupPartition {
    pub topic: String,
    pub partition: String,
    pub lag: Option<f64>,
    pub lag_seconds: Option<f64>,
    pub consumer_offset: Option<f64>,
    pub broker_offset: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KafkaGroupTopic {
    pub topic: String,
    /// Messages behind, all partitions together.
    pub lag: f64,
    /// The largest estimated lag in time of a partition (s).
    pub lag_seconds: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaConsumerGroup {
    pub cluster: String,
    pub name: String,
    pub state: Option<String>,
    pub members: Option<u64>,
    /// Messages behind, all topics and partitions together.
    pub lag: f64,
    pub lag_max: f64,
    pub lag_seconds: Option<f64>,
    pub lag_high: bool,
    /// Messages per second committed (average).
    pub consume_rate: Option<f64>,
    pub topics: Vec<KafkaGroupTopic>,
    pub partitions: Vec<KafkaGroupPartition>,
}

#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaConnector {
    pub cluster: String,
    pub name: String,
    pub connector_type: Option<String>,
    pub class: Option<String>,
    pub state: Option<String>,
    pub tasks: u64,
    pub running: u64,
    pub failed: u64,
}

#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaClusterChart {
    pub cluster: String,
    /// `[timestamp ms, messages/s]`.
    pub messages_in: Vec<(i64, f64)>,
    pub under_replicated: Vec<(i64, f64)>,
    pub offline: Vec<(i64, f64)>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KafkaChange {
    pub ts: i64,
    pub title: String,
    /// The event's alert type (error, warning, info, success).
    pub status: String,
    pub cluster: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KafkaConsole {
    /// False when no Kafka metrics arrived in the range (show the setup guide).
    pub enabled: bool,
    pub clusters: Vec<KafkaCluster>,
    pub topics: Vec<KafkaTopic>,
    pub brokers: Vec<KafkaBroker>,
    pub consumer_groups: Vec<KafkaConsumerGroup>,
    pub connectors: Vec<KafkaConnector>,
    pub charts: Vec<KafkaClusterChart>,
    pub health: Vec<KafkaHealthCheck>,
    pub changes: Vec<KafkaChange>,
}

// ================================================================ aggregation

/// The cluster of each series: its tag, else the only named cluster, else `default`.
struct Clusters {
    fallback: String,
}

impl Clusters {
    fn new(series: &[Series]) -> Self {
        let named: BTreeSet<String> = series.iter().filter_map(|s| s.tag(CLUSTER_KEYS)).collect();
        let fallback = if named.len() == 1 { named.into_iter().next().unwrap_or_default() } else { DEFAULT_CLUSTER.to_string() };
        Clusters { fallback }
    }
    fn of(&self, s: &Series) -> String {
        s.tag(CLUSTER_KEYS).unwrap_or_else(|| self.fallback.clone())
    }
}

/// The first non-empty of `choices` (sources in order of preference).
fn first_points(choices: Vec<Vec<(i64, f64)>>) -> Vec<(i64, f64)> {
    choices.into_iter().find(|c| !c.is_empty()).unwrap_or_default()
}

/// Partitions whose in-sync replica count matches `bad(in_sync, replicas)`, per time (OpenTelemetry receiver).
fn partitions_where(series: &[&Series], bad: impl Fn(f64, f64) -> bool) -> Vec<(i64, f64)> {
    let mut replicas: HashMap<(String, String), &Series> = HashMap::new();
    for s in series.iter().filter(|s| s.metric() == PARTITION_REPLICAS) {
        if let (Some(t), Some(p)) = (s.tag(&["topic"]), s.tag(&["partition"])) {
            replicas.insert((t, p), s);
        }
    }
    let mut acc: BTreeMap<i64, f64> = BTreeMap::new();
    for s in series.iter().filter(|s| s.metric() == OTEL_IN_SYNC) {
        let (Some(t), Some(p)) = (s.tag(&["topic"]), s.tag(&["partition"])) else { continue };
        let Some(r) = replicas.get(&(t, p)) else { continue };
        let want: HashMap<i64, f64> = r.points.iter().copied().collect();
        for (ts, in_sync) in &s.points {
            if let Some(n) = want.get(ts) {
                *acc.entry(*ts).or_default() += if bad(*in_sync, *n) { 1.0 } else { 0.0 };
            }
        }
    }
    acc.into_iter().collect()
}

fn chart_for(cluster: &str, series: &[&Series]) -> KafkaClusterChart {
    let of = |m: &'static str| move |s: &&Series| s.metric() == m;
    let pts = |m: &'static str| sum_by_t(series.iter().copied().filter(of(m)), |s| s.points.clone());
    let growth = sum_by_t(series.iter().copied().filter(|s| s.metric() == BROKER_OFFSET || s.metric() == OTEL_PARTITION_OFFSET), Series::growth);
    KafkaClusterChart {
        cluster: cluster.to_string(),
        messages_in: first_points(vec![pts(MESSAGES_IN), pts(TOPIC_MESSAGES_IN), pts(TOPIC_MESSAGE_RATE), growth]),
        under_replicated: first_points(vec![pts(UNDER_REPLICATED_JMX), pts(PARTITION_UNDER_REPLICATED), partitions_where(series, |in_sync, n| in_sync < n)]),
        offline: first_points(vec![pts(OFFLINE_JMX), pts(PARTITION_OFFLINE), partitions_where(series, |in_sync, _| in_sync <= 0.0)]),
    }
}

fn round_u64(v: f64) -> u64 {
    v.max(0.0).round() as u64
}

fn topics_for(cluster: &str, series: &[&Series], groups: &[KafkaConsumerGroup]) -> Vec<KafkaTopic> {
    let mut by_topic: BTreeMap<String, Vec<&Series>> = BTreeMap::new();
    for s in series {
        if let Some(t) = s.tag(&["topic"]).filter(|t| !t.starts_with("__")) {
            by_topic.entry(t).or_default().push(s);
        }
    }
    by_topic
        .into_iter()
        .map(|(name, ss)| {
            let with = |m: &str| ss.iter().copied().filter(|s| s.metric() == m).collect::<Vec<_>>();
            let latest_sum = |m: &str| {
                let v = with(m);
                (!v.is_empty()).then(|| v.iter().filter_map(|s| s.latest()).sum::<f64>())
            };
            let mean_sum = |m: &str| {
                let v = with(m);
                (!v.is_empty()).then(|| v.iter().filter_map(|s| s.mean()).sum::<f64>())
            };
            let growth_sum = |ms: &[&str]| {
                let v: Vec<_> = ss.iter().filter(|s| ms.contains(&s.metric())).filter_map(|s| s.growth_rate()).collect();
                (!v.is_empty()).then(|| v.iter().sum::<f64>())
            };
            let partitions = latest_sum(TOPIC_PARTITIONS).map(round_u64).or_else(|| {
                let parts: BTreeSet<String> = ss.iter().filter(|s| [BROKER_OFFSET, OTEL_PARTITION_OFFSET, PARTITION_REPLICAS].contains(&s.metric())).filter_map(|s| s.tag(&["partition"])).collect();
                (!parts.is_empty()).then_some(parts.len() as u64)
            });
            let replication_factor = with(PARTITION_REPLICAS).iter().filter_map(|s| s.latest()).reduce(f64::max).map(round_u64);
            let under_replicated = latest_sum(PARTITION_UNDER_REPLICATED).map(round_u64).or_else(|| {
                let p = partitions_where(&ss, |in_sync, n| in_sync < n);
                p.last().map(|x| round_u64(x.1))
            });
            let in_groups: Vec<&KafkaConsumerGroup> = groups.iter().filter(|g| g.cluster == cluster && g.topics.iter().any(|t| t.topic == name)).collect();
            let consumer_lag_max = in_groups.iter().filter_map(|g| g.topics.iter().find(|t| t.topic == name)).map(|t| t.lag).reduce(f64::max);
            KafkaTopic {
                cluster: cluster.to_string(),
                partitions,
                replication_factor,
                under_replicated,
                messages_in: mean_sum(TOPIC_MESSAGES_IN).or_else(|| mean_sum(TOPIC_MESSAGE_RATE)).or_else(|| growth_sum(&[BROKER_OFFSET, OTEL_PARTITION_OFFSET])),
                messages_out: growth_sum(&[CONSUMER_OFFSET, OTEL_GROUP_OFFSET]),
                bytes_in: mean_sum(TOPIC_BYTES_IN),
                bytes_out: mean_sum(TOPIC_BYTES_OUT),
                size: latest_sum(TOPIC_SIZE),
                consumer_lag_max,
                consumer_groups: in_groups.iter().map(|g| g.name.clone()).collect(),
                name,
            }
        })
        .collect()
}

fn groups_for(cluster: &str, series: &[&Series]) -> Vec<KafkaConsumerGroup> {
    let mut by_group: BTreeMap<String, Vec<&Series>> = BTreeMap::new();
    for s in series.iter().filter(|s| [CONSUMER_LAG, OTEL_GROUP_LAG, CONSUMER_OFFSET, OTEL_GROUP_OFFSET, LAG_SECONDS, GROUP_MEMBERS].contains(&s.metric())) {
        if let Some(g) = s.tag(GROUP_KEYS) {
            by_group.entry(g).or_default().push(s);
        }
    }
    // Broker offsets per (topic, partition), to show next to the group's.
    let broker_offsets: HashMap<(String, String), f64> =
        series.iter().filter(|s| s.metric() == BROKER_OFFSET || s.metric() == OTEL_PARTITION_OFFSET).filter_map(|s| Some(((s.tag(&["topic"])?, s.tag(&["partition"])?), s.latest()?))).collect();
    by_group
        .into_iter()
        .map(|(name, ss)| {
            let mut parts: BTreeMap<(String, String), KafkaGroupPartition> = BTreeMap::new();
            let mut members = None;
            let mut state = None;
            let mut rates = Vec::new();
            for s in &ss {
                if state.is_none() {
                    state = s.tag(&["consumer_group_state"]);
                }
                if s.metric() == GROUP_MEMBERS {
                    members = s.latest().map(round_u64);
                    continue;
                }
                let (Some(topic), Some(partition)) = (s.tag(&["topic"]), s.tag(&["partition"])) else { continue };
                let p = parts.entry((topic.clone(), partition.clone())).or_insert_with(|| KafkaGroupPartition {
                    broker_offset: broker_offsets.get(&(topic.clone(), partition.clone())).copied(),
                    topic,
                    partition,
                    lag: None,
                    lag_seconds: None,
                    consumer_offset: None,
                });
                match s.metric() {
                    CONSUMER_LAG | OTEL_GROUP_LAG => p.lag = s.latest().map(|v| v.max(0.0)),
                    LAG_SECONDS => p.lag_seconds = s.latest(),
                    _ => {
                        p.consumer_offset = s.latest();
                        rates.extend(s.growth_rate());
                    }
                }
            }
            let partitions: Vec<KafkaGroupPartition> = parts.into_values().collect();
            let mut topics: BTreeMap<String, KafkaGroupTopic> = BTreeMap::new();
            for p in &partitions {
                let t = topics.entry(p.topic.clone()).or_insert_with(|| KafkaGroupTopic { topic: p.topic.clone(), lag: 0.0, lag_seconds: None });
                t.lag += p.lag.unwrap_or(0.0);
                if let Some(s) = p.lag_seconds {
                    t.lag_seconds = Some(t.lag_seconds.map_or(s, |x| x.max(s)));
                }
            }
            let topics: Vec<KafkaGroupTopic> = topics.into_values().collect();
            let lag = topics.iter().map(|t| t.lag).sum::<f64>();
            let lag_seconds = topics.iter().filter_map(|t| t.lag_seconds).reduce(f64::max);
            let lag_max = partitions.iter().filter_map(|p| p.lag).fold(0.0, f64::max);
            KafkaConsumerGroup {
                cluster: cluster.to_string(),
                name,
                state,
                members,
                lag_high: lag_is_high(Some(topics.iter().map(|t| t.lag).fold(0.0, f64::max)), lag_seconds),
                lag,
                lag_max,
                lag_seconds,
                consume_rate: (!rates.is_empty()).then(|| rates.iter().sum()),
                topics,
                partitions,
            }
        })
        .collect()
}

fn brokers_for(cluster: &str, series: &[&Series]) -> Vec<KafkaBroker> {
    let mut brokers: BTreeMap<String, KafkaBroker> = BTreeMap::new();
    // The kafka_consumer check's cluster metadata: per broker id.
    for s in series.iter().filter(|s| s.metric() == BROKER_PARTITIONS || s.metric() == BROKER_LEADERS) {
        let Some(id) = s.tag(&["broker_id"]) else { continue };
        let b = brokers.entry(id.clone()).or_insert_with(|| KafkaBroker { cluster: cluster.to_string(), id, host: s.tag(&["broker_host"]), ..Default::default() });
        let v = s.latest().map(round_u64);
        if s.metric() == BROKER_PARTITIONS {
            b.partitions = v;
        } else {
            b.leaders = v;
        }
    }
    // The kafka check's JMX metrics: per broker host.
    const JMX: &[&str] = &[MESSAGES_IN, BYTES_IN, BYTES_OUT, UNDER_REPLICATED_JMX, ACTIVE_CONTROLLER, BROKER_PARTITIONS_JMX, BROKER_LEADERS_JMX];
    for s in series.iter().filter(|s| JMX.contains(&s.metric())) {
        let host = s.tag(&["host.name", "host"]);
        let id = s
            .tag(&["broker_id"])
            .or_else(|| host.as_ref().and_then(|h| brokers.values().find(|b| b.host.as_deref().is_some_and(|bh| bh == h || bh.starts_with(&format!("{h}.")))).map(|b| b.id.clone())))
            .or_else(|| host.clone());
        let Some(id) = id else { continue };
        let b = brokers.entry(id.clone()).or_insert_with(|| KafkaBroker { cluster: cluster.to_string(), id, ..Default::default() });
        if b.host.is_none() {
            b.host = host;
        }
        let add = |slot: &mut Option<f64>, v: Option<f64>| {
            if let Some(v) = v {
                *slot = Some(slot.unwrap_or(0.0) + v);
            }
        };
        match s.metric() {
            MESSAGES_IN => add(&mut b.messages_in, s.mean()),
            BYTES_IN => add(&mut b.bytes_in, s.mean()),
            BYTES_OUT => add(&mut b.bytes_out, s.mean()),
            UNDER_REPLICATED_JMX => add(&mut b.under_replicated, s.latest()),
            ACTIVE_CONTROLLER => b.controller = s.latest().is_some_and(|v| v >= 1.0),
            BROKER_PARTITIONS_JMX => b.partitions = b.partitions.or(s.latest().map(round_u64)),
            _ => b.leaders = b.leaders.or(s.latest().map(round_u64)),
        }
    }
    // The controller the kafka_consumer check names.
    for s in series.iter().filter(|s| s.metric() == CONTROLLER_ID) {
        if let Some(id) = s.latest().map(|v| round_u64(v).to_string()) {
            if let Some(b) = brokers.get_mut(&id) {
                b.controller = true;
            }
        }
    }
    brokers.into_values().collect()
}

fn connectors_for(cluster: &str, series: &[&Series]) -> Vec<KafkaConnector> {
    let mut out: BTreeMap<String, KafkaConnector> = BTreeMap::new();
    for s in series.iter().filter(|s| [CONNECTOR_TASK_COUNT, CONNECTOR_TASKS, CONNECT_TOTAL, CONNECT_RUNNING, CONNECT_FAILED].contains(&s.metric())) {
        let Some(name) = s.tag(&["connector"]) else { continue };
        let c = out.entry(name.clone()).or_insert_with(|| KafkaConnector { cluster: cluster.to_string(), name, ..Default::default() });
        let n = s.latest().map(round_u64).unwrap_or(0);
        for (slot, key) in [(&mut c.connector_type, "connector_type"), (&mut c.class, "connector_class"), (&mut c.state, "connector_state")] {
            if slot.is_none() {
                *slot = s.tag(&[key]);
            }
        }
        match s.metric() {
            CONNECTOR_TASK_COUNT | CONNECT_TOTAL => c.tasks = n,
            CONNECT_RUNNING => c.running = n,
            CONNECT_FAILED => c.failed = n,
            _ => match s.tag(&["task_state"]).as_deref() {
                Some("running") => c.running = n,
                Some("failed") => c.failed = n,
                _ => {}
            },
        }
    }
    out.into_values().collect()
}

fn checks_for(cluster: &KafkaCluster, series: &[&Series], groups: &[KafkaConsumerGroup], connectors: &[KafkaConnector]) -> Vec<KafkaHealthCheck> {
    let check = |name: &str, status: KafkaCheckStatus, detail: String| KafkaHealthCheck { cluster: cluster.name.clone(), name: name.to_string(), status, detail };
    let count_check = |name: &str, value: Option<f64>, what: &str| match value {
        None => check(name, KafkaCheckStatus::NoData, format!("No {what} metrics")),
        Some(v) if v > 0.0 => check(name, KafkaCheckStatus::Failing, format!("{} {what}", round_u64(v))),
        Some(_) => check(name, KafkaCheckStatus::Passing, format!("0 {what}")),
    };
    let mut out =
        vec![count_check("No offline partitions", cluster.offline, "offline partitions"), count_check("No under-replicated partitions", cluster.under_replicated, "under-replicated partitions")];
    let controllers: Vec<f64> = series.iter().filter(|s| s.metric() == ACTIVE_CONTROLLER).filter_map(|s| s.latest()).collect();
    out.push(if !controllers.is_empty() {
        let n = controllers.iter().sum::<f64>().round();
        let status = if n == 1.0 { KafkaCheckStatus::Passing } else { KafkaCheckStatus::Failing };
        check("One active controller", status, format!("{n} active controllers"))
    } else if series.iter().any(|s| s.metric() == CONTROLLER_ID) {
        check("One active controller", KafkaCheckStatus::Passing, "Controller elected".into())
    } else {
        check("One active controller", KafkaCheckStatus::NoData, "No controller metrics".into())
    });
    let mine: Vec<&KafkaConsumerGroup> = groups.iter().filter(|g| g.cluster == cluster.name).collect();
    out.push(if mine.is_empty() {
        check("Consumers keeping up", KafkaCheckStatus::NoData, "No consumer lag metrics".into())
    } else {
        let behind: Vec<&str> = mine.iter().filter(|g| g.lag_high).map(|g| g.name.as_str()).collect();
        if behind.is_empty() {
            check("Consumers keeping up", KafkaCheckStatus::Passing, format!("{} consumer groups within lag limits", mine.len()))
        } else {
            check("Consumers keeping up", KafkaCheckStatus::Failing, format!("High lag: {}", behind.join(", ")))
        }
    });
    let conns: Vec<&KafkaConnector> = connectors.iter().filter(|c| c.cluster == cluster.name).collect();
    if !conns.is_empty() {
        let failed: Vec<&str> = conns.iter().filter(|c| c.failed > 0 || c.state.as_deref() == Some("failed")).map(|c| c.name.as_str()).collect();
        out.push(if failed.is_empty() {
            check("Connectors running", KafkaCheckStatus::Passing, format!("{} connectors", conns.len()))
        } else {
            check("Connectors running", KafkaCheckStatus::Failing, format!("Failed: {}", failed.join(", ")))
        });
    }
    out
}

/// The console behind the interval values of [`ALL_METRICS`] (monitors and changes are filled in later).
pub(crate) fn aggregate(values: &[SeriesValue]) -> KafkaConsole {
    let series = group_series(values);
    if series.is_empty() {
        return KafkaConsole::default();
    }
    let clusters = Clusters::new(&series);
    let mut by_cluster: BTreeMap<String, Vec<&Series>> = BTreeMap::new();
    for s in &series {
        by_cluster.entry(clusters.of(s)).or_default().push(s);
    }
    let mut out = KafkaConsole { enabled: true, ..Default::default() };
    for (name, ss) in &by_cluster {
        let chart = chart_for(name, ss);
        let groups = groups_for(name, ss);
        let topics = topics_for(name, ss, &groups);
        let brokers = brokers_for(name, ss);
        let connectors = connectors_for(name, ss);
        let latest = |m: &str| ss.iter().filter(|s| s.metric() == m).filter_map(|s| s.latest()).reduce(f64::max).map(round_u64);
        let partitions: Vec<u64> = topics.iter().filter_map(|t| t.partitions).collect();
        let mut cluster = KafkaCluster {
            name: name.clone(),
            brokers: latest(BROKER_COUNT).or_else(|| latest(OTEL_BROKERS)).or_else(|| (!brokers.is_empty()).then_some(brokers.len() as u64)),
            topics: topics.len() as u64,
            partitions: (!partitions.is_empty()).then(|| partitions.iter().sum()),
            under_replicated: chart.under_replicated.last().map(|p| p.1),
            offline: chart.offline.last().map(|p| p.1),
            messages_in: mean_of(&chart.messages_in),
            consumer_groups: groups.len() as u64,
            connectors: connectors.len() as u64,
            ..Default::default()
        };
        let checks = checks_for(&cluster, ss, &groups, &connectors);
        cluster.checks_passing = checks.iter().filter(|c| c.status == KafkaCheckStatus::Passing).count() as u64;
        cluster.checks_failing = checks.iter().filter(|c| c.status == KafkaCheckStatus::Failing).count() as u64;
        out.clusters.push(cluster);
        out.charts.push(chart);
        out.health.extend(checks);
        out.topics.extend(topics);
        out.brokers.extend(brokers);
        out.consumer_groups.extend(groups);
        out.connectors.extend(connectors);
    }
    out
}

/// Monitors (their queries) that watch `cluster`: they name Kafka and the cluster.
pub(crate) fn monitors_watching(queries: &[String], cluster: &str) -> u64 {
    let c = cluster.to_lowercase();
    queries
        .iter()
        .filter(|q| {
            let q = q.to_lowercase();
            q.contains("kafka") && q.contains(&c)
        })
        .count() as u64
}

/// The cluster an event's tags (`k:v,k:v`) name.
fn event_cluster(tags: &str) -> Option<String> {
    let tags: HashMap<&str, &str> = tags.split(',').filter_map(|t| t.split_once(':')).collect();
    CLUSTER_KEYS.iter().find_map(|k| tags.get(k).filter(|v| !v.is_empty()).map(|v| v.to_string()))
}

// ================================================================ handler

#[derive(Deserialize, ToSchema)]
pub struct KafkaConsoleRequest {
    pub range: TimeRange,
}

#[derive(Row, Deserialize)]
struct ChangeRow {
    ts: i64,
    title: String,
    status: String,
    tags: String,
}

/// Kafka events in the range: what the Agent's Kafka checks report and events tagged with a cluster.
async fn changes(st: &AppState, org: &str, range: &TimeRange) -> Result<Vec<KafkaChange>, ApiError> {
    let rows = st
        .ch(org)
        .query(
            "SELECT toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, LogAttributes['title'] AS title, toString(SeverityText) AS status, LogAttributes['tags'] AS tags \
             FROM otel.otel_events WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND (positionCaseInsensitive(LogAttributes['source_type_name'], 'kafka') > 0 OR position(LogAttributes['tags'], 'kafka_cluster') > 0) \
             ORDER BY Timestamp DESC LIMIT 100",
        )
        .bind(range.from)
        .bind(range.to)
        .fetch_all::<ChangeRow>()
        .await?;
    Ok(rows.into_iter().map(|r| KafkaChange { ts: r.ts, cluster: event_cluster(&r.tags), title: r.title, status: r.status }).collect())
}

/// Kafka clusters, topics, brokers, consumer groups and connectors that reported in the range.
#[utoipa::path(post, path = "/api/v1/data-streams/kafka", operation_id = "kafka_console", request_body = KafkaConsoleRequest, responses((status = 200, body = KafkaConsole)), tag = "data-streams")]
pub async fn console(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<KafkaConsoleRequest>) -> ApiResult<KafkaConsole> {
    req.range.validate()?;
    let step = chart_step(req.range.to - req.range.from, 60);
    let values = series_values(&st, &org, ALL_METRICS, req.range.from, req.range.to, Some(step)).await?;
    let mut page = aggregate(&values);
    if !page.enabled {
        return Ok(Json(page));
    }
    let queries: Vec<(String,)> = sqlx::query_as("SELECT query FROM monitors WHERE org_id = $1 AND query ILIKE '%kafka%' LIMIT 5000").bind(&org).fetch_all(&st.postgres).await?;
    let queries: Vec<String> = queries.into_iter().map(|q| q.0).collect();
    for c in &mut page.clusters {
        c.monitors = monitors_watching(&queries, &c.name);
    }
    page.changes = changes(&st, &org, &req.range).await?;
    Ok(Json(page))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn sv(metric: &str, t: i64, value: f64, res: &[(&str, &str)], attrs: &[(&str, &str)]) -> SeriesValue {
        let map = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        SeriesValue { metric: metric.into(), res: map(res), attrs: map(attrs), t, value, max: value, count: 1, last: t }
    }

    #[test]
    fn datadog_checks_make_a_cluster_with_brokers_topics_and_groups() {
        let shop = ("kafka_cluster", "shop-events");
        let b1 = [("host.name", "i-0a1b2c3d4e5f60011")];
        let b2 = [("host.name", "i-0a1b2c3d4e5f60012")];
        let part = |p: &'static str| [shop, ("topic", "orders.created"), ("partition", p)];
        let grp = |p: &'static str| [shop, ("topic", "orders.created"), ("partition", p), ("consumer_group", "billing-consumer")];
        let values = vec![
            sv(MESSAGES_IN, 0, 100.0, &b1, &[shop]),
            sv(MESSAGES_IN, MIN, 120.0, &b1, &[shop]),
            sv(MESSAGES_IN, 0, 40.0, &b2, &[shop]),
            sv(MESSAGES_IN, MIN, 40.0, &b2, &[shop]),
            sv(ACTIVE_CONTROLLER, MIN, 1.0, &b1, &[shop]),
            sv(ACTIVE_CONTROLLER, MIN, 0.0, &b2, &[shop]),
            sv(UNDER_REPLICATED_JMX, MIN, 0.0, &b1, &[shop]),
            sv(OFFLINE_JMX, MIN, 0.0, &b1, &[shop]),
            sv(BROKER_OFFSET, 0, 1_000.0, &[], &part("0")),
            sv(BROKER_OFFSET, MIN, 1_600.0, &[], &part("0")),
            sv(BROKER_OFFSET, MIN, 500.0, &[], &part("1")),
            sv(CONSUMER_OFFSET, 0, 900.0, &[], &grp("0")),
            sv(CONSUMER_OFFSET, MIN, 1_500.0, &[], &grp("0")),
            sv(CONSUMER_LAG, MIN, 100.0, &[], &grp("0")),
            sv(CONSUMER_LAG, MIN, 30.0, &[], &grp("1")),
            sv(LAG_SECONDS, MIN, 4.0, &[], &grp("0")),
        ];
        let page = aggregate(&values);
        assert!(page.enabled);
        assert_eq!(page.clusters.len(), 1);
        let c = &page.clusters[0];
        assert_eq!(c.name, "shop-events");
        assert_eq!(c.brokers, Some(2), "one broker per JMX host");
        assert_eq!(c.topics, 1);
        assert_eq!(c.partitions, Some(2), "distinct partitions of the offsets");
        assert_eq!(c.messages_in, Some(150.0), "the brokers' sum, averaged over the range");
        assert_eq!((c.under_replicated, c.offline), (Some(0.0), Some(0.0)));
        assert_eq!(c.consumer_groups, 1);
        let t = &page.topics[0];
        assert_eq!(t.messages_in, Some(10.0), "offset growth: 600 messages in a minute");
        assert_eq!(t.messages_out, Some(10.0));
        assert_eq!(t.consumer_lag_max, Some(130.0));
        assert_eq!(t.consumer_groups, vec!["billing-consumer".to_string()]);
        let g = &page.consumer_groups[0];
        assert_eq!((g.lag, g.lag_max, g.lag_seconds), (130.0, 100.0, Some(4.0)));
        assert!(!g.lag_high);
        assert_eq!(g.partitions[0].broker_offset, Some(1_600.0));
        assert_eq!(g.partitions[0].consumer_offset, Some(1_500.0));
        let controller: Vec<&KafkaBroker> = page.brokers.iter().filter(|b| b.controller).collect();
        assert_eq!(controller.len(), 1);
        assert_eq!(controller[0].id, "i-0a1b2c3d4e5f60011");
        assert_eq!(c.checks_failing, 0);
        assert_eq!(c.checks_passing, 4, "offline, under-replicated, controller and lag");
    }

    #[test]
    fn opentelemetry_receiver_names_are_read_too() {
        let cl = ("kafka.cluster.alias", "shop-events");
        let p = |n: &'static str| [("topic", "payments.authorized"), ("partition", n)];
        let values = vec![
            sv(OTEL_BROKERS, MIN, 3.0, &[cl], &[]),
            sv(TOPIC_PARTITIONS, MIN, 3.0, &[cl], &[("topic", "payments.authorized")]),
            sv(PARTITION_REPLICAS, MIN, 3.0, &[cl], &p("0")),
            sv(OTEL_IN_SYNC, MIN, 3.0, &[cl], &p("0")),
            sv(PARTITION_REPLICAS, MIN, 3.0, &[cl], &p("1")),
            sv(OTEL_IN_SYNC, MIN, 2.0, &[cl], &p("1")),
            sv(PARTITION_REPLICAS, MIN, 3.0, &[cl], &p("2")),
            sv(OTEL_IN_SYNC, MIN, 0.0, &[cl], &p("2")),
            sv(OTEL_GROUP_LAG, MIN, 20_000.0, &[cl], &[("group", "order-consumer"), ("topic", "payments.authorized"), ("partition", "0")]),
            sv(GROUP_MEMBERS, MIN, 4.0, &[cl], &[("group", "order-consumer")]),
        ];
        let page = aggregate(&values);
        let c = &page.clusters[0];
        assert_eq!(c.name, "shop-events");
        assert_eq!(c.brokers, Some(3));
        assert_eq!(c.partitions, Some(3));
        assert_eq!(c.under_replicated, Some(2.0), "fewer in-sync replicas than replicas");
        assert_eq!(c.offline, Some(1.0), "no in-sync replica");
        let t = &page.topics[0];
        assert_eq!((t.replication_factor, t.under_replicated), (Some(3), Some(2)));
        let g = &page.consumer_groups[0];
        assert_eq!((g.name.as_str(), g.members, g.lag), ("order-consumer", Some(4), 20_000.0));
        assert!(g.lag_high, "no lag in time: the message count decides");
        let failing: Vec<&str> = page.health.iter().filter(|h| h.status == KafkaCheckStatus::Failing).map(|h| h.name.as_str()).collect();
        assert_eq!(failing, vec!["No offline partitions", "No under-replicated partitions", "Consumers keeping up"]);
        assert!(page.health.iter().any(|h| h.name == "One active controller" && h.status == KafkaCheckStatus::NoData));
    }

    #[test]
    fn charts_prefer_broker_metrics_then_fall_back() {
        let a = ("kafka_cluster_id", "a1");
        let values = vec![
            sv(TOPIC_MESSAGE_RATE, 0, 5.0, &[], &[a, ("topic", "inventory.reserved")]),
            sv(TOPIC_MESSAGE_RATE, MIN, 7.0, &[], &[a, ("topic", "inventory.reserved")]),
            sv(PARTITION_UNDER_REPLICATED, MIN, 1.0, &[], &[a, ("topic", "inventory.reserved"), ("partition", "0")]),
            sv(PARTITION_OFFLINE, MIN, 0.0, &[], &[a, ("topic", "inventory.reserved"), ("partition", "0")]),
        ];
        let page = aggregate(&values);
        let chart = &page.charts[0];
        assert_eq!(chart.messages_in, vec![(0, 5.0), (MIN, 7.0)]);
        assert_eq!(chart.under_replicated, vec![(MIN, 1.0)]);
        assert_eq!(page.topics[0].messages_in, Some(6.0));
        assert_eq!(page.topics[0].under_replicated, Some(1));
    }

    #[test]
    fn untagged_series_join_the_only_named_cluster() {
        let values = vec![sv(MESSAGES_IN, MIN, 10.0, &[("host.name", "i-0a1b2c3d4e5f60011")], &[]), sv(BROKER_COUNT, MIN, 3.0, &[], &[("kafka_cluster_id", "shop-events")])];
        let page = aggregate(&values);
        assert_eq!(page.clusters.len(), 1);
        assert_eq!(page.clusters[0].brokers, Some(3), "the check's broker count wins");
        let two = aggregate(&[values[0].clone(), values[1].clone(), sv(BROKER_COUNT, MIN, 1.0, &[], &[("kafka_cluster_id", "analytics")])]);
        assert_eq!(two.clusters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["analytics", "default", "shop-events"]);
    }

    #[test]
    fn brokers_from_the_check_metadata_merge_with_jmx_hosts() {
        let cl = ("kafka_cluster_id", "shop-events");
        let values = vec![
            sv(BROKER_PARTITIONS, MIN, 24.0, &[], &[cl, ("broker_id", "1"), ("broker_host", "i-0a1b2c3d4e5f60011.ec2.internal")]),
            sv(BROKER_LEADERS, MIN, 8.0, &[], &[cl, ("broker_id", "1"), ("broker_host", "i-0a1b2c3d4e5f60011.ec2.internal")]),
            sv(CONTROLLER_ID, MIN, 1.0, &[], &[cl]),
            sv(BYTES_IN, MIN, 2048.0, &[("host.name", "i-0a1b2c3d4e5f60011")], &[cl]),
        ];
        let page = aggregate(&values);
        assert_eq!(page.brokers.len(), 1);
        let b = &page.brokers[0];
        assert_eq!((b.id.as_str(), b.partitions, b.leaders, b.bytes_in, b.controller), ("1", Some(24), Some(8), Some(2048.0), true));
        assert!(page.health.iter().any(|h| h.name == "One active controller" && h.status == KafkaCheckStatus::Passing));
    }

    #[test]
    fn connectors_from_the_consumer_check_and_connect_workers() {
        let cl = ("kafka_cluster_id", "shop-events");
        let c = [cl, ("connector", "orders-s3-sink"), ("connector_type", "sink"), ("connector_state", "running")];
        let values = vec![
            sv(CONNECTOR_TASK_COUNT, MIN, 2.0, &[], &c),
            sv(CONNECTOR_TASKS, MIN, 1.0, &[], &[c[0], c[1], c[2], c[3], ("task_state", "running")]),
            sv(CONNECTOR_TASKS, MIN, 1.0, &[], &[c[0], c[1], c[2], c[3], ("task_state", "failed")]),
            sv(CONNECT_TOTAL, MIN, 3.0, &[], &[cl, ("connector", "catalog-cdc-source")]),
            sv(CONNECT_RUNNING, MIN, 3.0, &[], &[cl, ("connector", "catalog-cdc-source")]),
        ];
        let page = aggregate(&values);
        assert_eq!(page.connectors.len(), 2);
        let sink = page.connectors.iter().find(|c| c.name == "orders-s3-sink").unwrap();
        assert_eq!((sink.tasks, sink.running, sink.failed, sink.connector_type.as_deref()), (2, 1, 1, Some("sink")));
        let cdc = page.connectors.iter().find(|c| c.name == "catalog-cdc-source").unwrap();
        assert_eq!((cdc.tasks, cdc.running, cdc.failed), (3, 3, 0));
        assert!(page.health.iter().any(|h| h.name == "Connectors running" && h.status == KafkaCheckStatus::Failing && h.detail.contains("orders-s3-sink")));
    }

    #[test]
    fn nothing_reported_means_disabled() {
        assert!(!aggregate(&[]).enabled);
    }

    #[test]
    fn lag_thresholds_prefer_time() {
        assert!(!lag_is_high(Some(50_000.0), Some(10.0)), "lag in time decides when known");
        assert!(lag_is_high(Some(10.0), Some(61.0)));
        assert!(lag_is_high(Some(10_001.0), None));
        assert!(!lag_is_high(None, None));
    }

    #[test]
    fn monitors_and_events_name_the_cluster() {
        let queries = vec![
            "avg(last_5m):max:kafka.consumer_lag{kafka_cluster:shop-events} > 1000".to_string(),
            "avg(last_5m):avg:system.cpu.user{*} > 90".to_string(),
            "max:kafka.broker_offset{kafka_cluster:analytics}".to_string(),
        ];
        assert_eq!(monitors_watching(&queries, "shop-events"), 1);
        assert_eq!(monitors_watching(&queries, "Analytics"), 1);
        assert_eq!(event_cluster("topic:orders.created,kafka_cluster_id:shop-events"), Some("shop-events".into()));
        assert_eq!(event_cluster("env:prod"), None);
    }

    #[test]
    fn growth_skips_resets() {
        let s = Series { label: sv(BROKER_OFFSET, 0, 0.0, &[], &[]), points: vec![(0, 100.0), (MIN, 700.0), (2 * MIN, 50.0), (3 * MIN, 650.0)] };
        assert_eq!(s.growth(), vec![(MIN, 10.0), (3 * MIN, 10.0)]);
        assert_eq!(s.growth_rate(), Some(10.0));
    }
}
