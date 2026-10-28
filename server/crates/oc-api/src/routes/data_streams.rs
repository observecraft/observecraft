//! Data Streams Monitoring: services and the queues / topics between them, from APM spans.
//!
//! A messaging span is one with a messaging system (`messaging.system`, or a Datadog tracer's
//! `kafka.*` / `rabbitmq.*` operation), a destination (`messaging.destination.name`,
//! `messaging.destination`, `kafka.topic`, `topic`) and a role: producer / consumer span kind,
//! `messaging.operation[.type]` (publish, send, create / receive, process, deliver, settle), or
//! the Datadog operations `kafka.produce` / `kafka.consume`.
//!
//! - **Edges**: service → topic (spans that publish) and topic → service (spans that consume), with
//!   throughput, errors and span durations; the consumer group comes from
//!   `messaging.consumer.group.name`, `messaging.kafka.consumer.group`, `kafka.group` …
//! - **Hops**: a consume span whose parent or span link is a produce span gives one message's
//!   producer → topic → consumer, and its latency (consume start − produce start). Spans are
//!   grouped on that span id, so no join is needed and it works sharded too.
//! - **Pathways**: chains of hops, from services that only produce to services that produce
//!   nothing further. Their latency adds up the hops' (time spent inside services between hops
//!   is not included).
//! - **Consumer lag** from the Kafka metrics (see `kafka_console`), joined on topic and group.
//!
//! Tenancy: every query filters on `OrgId = {oc_org:String}` through `AppState::ch(org)`.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use axum::extract::State;
use axum::Json;
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::kafka_console::{self, group_series, lag_is_high, KafkaConsumerGroup, LAG_METRICS};
use super::logs::TimeRange;
use super::metric_inventory::{chart_step, series_values};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const SYSTEM: &str = "multiIf(SpanAttributes['messaging.system'] != '', SpanAttributes['messaging.system'], \
     startsWith(SpanAttributes['operation.name'], 'kafka.'), 'kafka', \
     startsWith(SpanAttributes['operation.name'], 'rabbitmq.') OR startsWith(SpanAttributes['operation.name'], 'amqp.'), 'rabbitmq', '')";
const DEST: &str = "arrayFirst(x -> x != '', [SpanAttributes['messaging.destination.name'], SpanAttributes['messaging.destination'], SpanAttributes['kafka.topic'], SpanAttributes['topic'], ''])";
const OPERATION: &str = "arrayFirst(x -> x != '', [SpanAttributes['messaging.operation.type'], SpanAttributes['messaging.operation'], ''])";
const GROUP: &str = "arrayFirst(x -> x != '', [SpanAttributes['messaging.consumer.group.name'], SpanAttributes['messaging.kafka.consumer.group'], \
     SpanAttributes['messaging.kafka.consumer_group'], SpanAttributes['kafka.group'], SpanAttributes['kafka.group_id'], ''])";

fn role_expr() -> String {
    format!(
        "multiIf(SpanKind = 'Producer', 'produce', SpanKind = 'Consumer', 'consume', \
         {OPERATION} IN ('publish', 'send', 'create'), 'produce', {OPERATION} IN ('receive', 'process', 'deliver', 'settle'), 'consume', \
         SpanAttributes['operation.name'] = 'kafka.produce', 'produce', SpanAttributes['operation.name'] = 'kafka.consume', 'consume', '')"
    )
}

/// The time window and optional env (`?` parameters: from, to[, env]).
fn window(env: bool) -> &'static str {
    if env {
        "Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND ResourceAttributes['deployment.environment'] = ?"
    } else {
        "Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)"
    }
}

/// Messaging spans per service, system, topic, role and consumer group.
pub(crate) fn endpoints_sql(env: bool) -> String {
    format!(
        "SELECT toString(ServiceName) AS service, {SYSTEM} AS system, {DEST} AS topic, {role} AS role, {GROUP} AS grp, \
         count() AS spans, countIf(StatusCode = 'Error') AS errors, quantile(0.5)(Duration) / 1e6 AS p50_ms, quantile(0.99)(Duration) / 1e6 AS p99_ms \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND system != '' AND topic != '' AND role != '' \
         GROUP BY service, system, topic, role, grp ORDER BY spans DESC LIMIT 5000",
        role = role_expr(),
        w = window(env),
    )
}

/// Linked producer → consumer hops with their latency, per interval of `step_ms` (whole range when none).
pub(crate) fn hops_sql(env: bool, step_ms: Option<i64>) -> String {
    let t = match step_ms {
        Some(step) => format!("intDiv(intDiv(tupleElement(c, 2), 1000000), {step}) * {step}", step = step.max(1000)),
        None => "toInt64(0)".to_string(),
    };
    format!(
        "SELECT producer, consumer, system, topic, t, count() AS n, quantile(0.5)(lat) AS p50_ms, quantile(0.99)(lat) AS p99_ms FROM ( \
           SELECT producer, tupleElement(c, 1) AS consumer, tupleElement(c, 3) AS system, if(tupleElement(c, 4) != '', tupleElement(c, 4), ptopic) AS topic, \
                  toInt64({t}) AS t, greatest(tupleElement(c, 2) - pts, 0) / 1e6 AS lat \
           FROM ( \
             SELECT key, argMinIf(service, ts, role = 'produce') AS producer, minIf(ts, role = 'produce') AS pts, anyIf(topic, role = 'produce') AS ptopic, \
                    groupArrayIf((service, ts, system, topic), role = 'consume') AS cons \
             FROM ( \
               SELECT toString(ServiceName) AS service, {role} AS role, {SYSTEM} AS system, {DEST} AS topic, toUnixTimestamp64Nano(Timestamp) AS ts, \
                      arrayJoin(if(role = 'produce', [SpanId], arrayFilter(x -> x != '', arrayConcat([ParentSpanId], `Links.SpanId`)))) AS key \
               FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND system != '' AND role != '' \
             ) GROUP BY key HAVING countIf(role = 'produce') > 0 AND notEmpty(cons) \
           ) ARRAY JOIN cons AS c \
         ) WHERE topic != '' GROUP BY producer, consumer, system, topic, t ORDER BY t LIMIT 50000",
        role = role_expr(),
        w = window(env),
    )
}

/// Messages a service publishes and consumes (or a topic receives and delivers) per interval.
pub(crate) fn node_rate_sql(env: bool, step_ms: i64, topic: bool) -> String {
    let filter = if topic { "system = ? AND topic = ?" } else { "service = ?" };
    format!(
        "SELECT toInt64(intDiv(toUnixTimestamp64Milli(Timestamp), {step}) * {step}) AS t, countIf(role = 'produce') AS produced, countIf(role = 'consume') AS consumed FROM ( \
           SELECT Timestamp, toString(ServiceName) AS service, {role} AS role, {SYSTEM} AS system, {DEST} AS topic \
           FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {w} AND system != '' AND topic != '' AND role != '' \
         ) WHERE {filter} GROUP BY t ORDER BY t",
        step = step_ms.max(1000),
        role = role_expr(),
        w = window(env),
    )
}

#[derive(Row, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct EndpointRow {
    pub service: String,
    pub system: String,
    pub topic: String,
    pub role: String,
    pub grp: String,
    pub spans: u64,
    pub errors: u64,
    pub p50_ms: f64,
    pub p99_ms: f64,
}

#[derive(Row, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct HopRow {
    pub producer: String,
    pub consumer: String,
    pub system: String,
    pub topic: String,
    pub t: i64,
    pub n: u64,
    pub p50_ms: f64,
    pub p99_ms: f64,
}

pub(crate) fn service_id(name: &str) -> String {
    format!("service:{name}")
}

pub(crate) fn topic_id(system: &str, topic: &str) -> String {
    format!("topic:{system}:{topic}")
}

// ================================================================ response

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum StreamNodeKind {
    Service,
    Topic,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum StreamEdgeKind {
    Produce,
    Consume,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamNode {
    /// `service:<name>` or `topic:<system>:<name>`.
    pub id: String,
    pub kind: StreamNodeKind,
    pub name: String,
    /// The messaging system of a topic (kafka, rabbitmq, …).
    pub system: Option<String>,
    /// Messages per second a service publishes, or a topic receives.
    pub produced: f64,
    /// Messages per second a service consumes, or a topic delivers.
    pub consumed: f64,
    pub errors: u64,
    /// The largest consumer group lag on a topic, or on what a service consumes (messages).
    pub lag: Option<f64>,
    pub lag_seconds: Option<f64>,
    pub lag_high: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamEdge {
    pub source: String,
    pub target: String,
    pub kind: StreamEdgeKind,
    /// Messages per second.
    pub throughput: f64,
    pub errors: u64,
    /// Duration of the publish / consume spans.
    pub span_p50_ms: f64,
    pub span_p99_ms: f64,
    /// Consume edges: time from publish to consume, over the linked messages.
    pub latency_p50_ms: Option<f64>,
    pub latency_p99_ms: Option<f64>,
    pub consumer_groups: Vec<String>,
    pub lag: Option<f64>,
    pub lag_seconds: Option<f64>,
    pub lag_high: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamPathway {
    /// The node ids joined by `>`.
    pub id: String,
    /// Node ids from the first producer to the last consumer.
    pub nodes: Vec<String>,
    pub hops: u64,
    /// The hops' latencies added up.
    pub latency_p50_ms: f64,
    pub latency_p99_ms: f64,
    /// Messages per second reaching the last consumer.
    pub throughput: f64,
    /// Some consumer on the pathway lags behind.
    pub lag_high: bool,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamMap {
    /// False when no messaging spans arrived in the range (show the onboarding).
    pub enabled: bool,
    pub nodes: Vec<StreamNode>,
    pub edges: Vec<StreamEdge>,
    pub pathways: Vec<StreamPathway>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamLatencyPoint {
    pub t: i64,
    pub p50_ms: f64,
    pub p99_ms: f64,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamMeasure {
    /// Node ids from start to end; empty when no messages flow from one to the other.
    pub path: Vec<String>,
    pub latency_p50_ms: Option<f64>,
    pub latency_p99_ms: Option<f64>,
    /// Messages per second reaching the end.
    pub throughput: Option<f64>,
    pub series: Vec<StreamLatencyPoint>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamLagSeries {
    pub group: String,
    pub topic: String,
    /// `[timestamp ms, messages behind]`.
    pub points: Vec<(i64, f64)>,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamNodeDetail {
    /// `[timestamp ms, messages/s]` published by a service or received by a topic.
    pub produced: Vec<(i64, f64)>,
    /// Consumed by a service or delivered from a topic.
    pub consumed: Vec<(i64, f64)>,
    pub lag: Vec<StreamLagSeries>,
}

// ================================================================ graph

/// Hops of a range (t = 0) merged per producer, topic and consumer.
type HopKey = (String, String, String, String);

fn hop_key(h: &HopRow) -> HopKey {
    (h.producer.clone(), h.system.clone(), h.topic.clone(), h.consumer.clone())
}

/// Weighted by message count: the hops of several producers into one consumer edge.
#[derive(Default)]
struct Latency {
    n: u64,
    p50_sum: f64,
    p99: f64,
}

impl Latency {
    fn add(&mut self, h: &HopRow) {
        self.n += h.n;
        self.p50_sum += h.p50_ms * h.n as f64;
        self.p99 = self.p99.max(h.p99_ms);
    }
    fn p50(&self) -> Option<f64> {
        (self.n > 0).then(|| self.p50_sum / self.n as f64)
    }
}

/// The lag of `groups` (all groups when none) on `topic`: (largest messages, largest seconds).
fn lag_on(lags: &[KafkaConsumerGroup], topic: &str, groups: Option<&BTreeSet<String>>) -> (Option<f64>, Option<f64>) {
    let mut msgs = None::<f64>;
    let mut secs = None::<f64>;
    for g in lags.iter().filter(|g| groups.is_none_or(|w| w.contains(&g.name))) {
        for t in g.topics.iter().filter(|t| t.topic == topic) {
            msgs = Some(msgs.map_or(t.lag, |m| m.max(t.lag)));
            if let Some(s) = t.lag_seconds {
                secs = Some(secs.map_or(s, |x| x.max(s)));
            }
        }
    }
    (msgs, secs)
}

fn max_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    }
}

/// The map: nodes, edges and pathways of a range of `window_s` seconds.
pub(crate) fn build_map(endpoints: &[EndpointRow], hops: &[HopRow], lags: &[KafkaConsumerGroup], window_s: f64) -> StreamMap {
    let per_s = |n: u64| n as f64 / window_s.max(1.0);
    struct EdgeAcc {
        kind: StreamEdgeKind,
        system: String,
        topic: String,
        spans: u64,
        errors: u64,
        p50_sum: f64,
        p99: f64,
        groups: BTreeSet<String>,
    }
    let mut edges: BTreeMap<(String, String), EdgeAcc> = BTreeMap::new();
    for e in endpoints {
        let (kind, source, target) = match e.role.as_str() {
            "produce" => (StreamEdgeKind::Produce, service_id(&e.service), topic_id(&e.system, &e.topic)),
            "consume" => (StreamEdgeKind::Consume, topic_id(&e.system, &e.topic), service_id(&e.service)),
            _ => continue,
        };
        let a =
            edges.entry((source, target)).or_insert_with(|| EdgeAcc { kind, system: e.system.clone(), topic: e.topic.clone(), spans: 0, errors: 0, p50_sum: 0.0, p99: 0.0, groups: BTreeSet::new() });
        a.spans += e.spans;
        a.errors += e.errors;
        a.p50_sum += e.p50_ms * e.spans as f64;
        a.p99 = a.p99.max(e.p99_ms);
        if !e.grp.is_empty() {
            a.groups.insert(e.grp.clone());
        }
    }
    let mut latency: HashMap<(String, String), Latency> = HashMap::new();
    for h in hops {
        latency.entry((topic_id(&h.system, &h.topic), service_id(&h.consumer))).or_default().add(h);
    }
    let edges: Vec<StreamEdge> = edges
        .into_iter()
        .map(|((source, target), a)| {
            let l = latency.get(&(source.clone(), target.clone()));
            let (lag, lag_seconds) = if a.kind == StreamEdgeKind::Consume && a.system == "kafka" && !a.groups.is_empty() { lag_on(lags, &a.topic, Some(&a.groups)) } else { (None, None) };
            StreamEdge {
                kind: a.kind,
                throughput: per_s(a.spans),
                errors: a.errors,
                span_p50_ms: if a.spans > 0 { a.p50_sum / a.spans as f64 } else { 0.0 },
                span_p99_ms: a.p99,
                latency_p50_ms: l.and_then(Latency::p50),
                latency_p99_ms: l.map(|l| l.p99),
                consumer_groups: a.groups.into_iter().collect(),
                lag_high: lag_is_high(lag, lag_seconds),
                lag,
                lag_seconds,
                source,
                target,
            }
        })
        .collect();

    let mut nodes: BTreeMap<String, StreamNode> = BTreeMap::new();
    for e in endpoints {
        let sid = service_id(&e.service);
        nodes.entry(sid.clone()).or_insert_with(|| StreamNode {
            id: sid,
            kind: StreamNodeKind::Service,
            name: e.service.clone(),
            system: None,
            produced: 0.0,
            consumed: 0.0,
            errors: 0,
            lag: None,
            lag_seconds: None,
            lag_high: false,
        });
        let tid = topic_id(&e.system, &e.topic);
        nodes.entry(tid.clone()).or_insert_with(|| {
            let (lag, lag_seconds) = if e.system == "kafka" { lag_on(lags, &e.topic, None) } else { (None, None) };
            StreamNode {
                id: tid,
                kind: StreamNodeKind::Topic,
                name: e.topic.clone(),
                system: Some(e.system.clone()),
                produced: 0.0,
                consumed: 0.0,
                errors: 0,
                lag_high: lag_is_high(lag, lag_seconds),
                lag,
                lag_seconds,
            }
        });
    }
    for e in &edges {
        let (svc, topic) = if e.kind == StreamEdgeKind::Produce { (&e.source, &e.target) } else { (&e.target, &e.source) };
        if let Some(n) = nodes.get_mut(svc) {
            n.errors += e.errors;
            if e.kind == StreamEdgeKind::Produce {
                n.produced += e.throughput;
            } else {
                n.consumed += e.throughput;
                n.lag = max_opt(n.lag, e.lag);
                n.lag_seconds = max_opt(n.lag_seconds, e.lag_seconds);
                n.lag_high |= e.lag_high;
            }
        }
        if let Some(n) = nodes.get_mut(topic) {
            if e.kind == StreamEdgeKind::Produce {
                n.produced += e.throughput;
            } else {
                n.consumed += e.throughput;
            }
        }
    }
    let pathways = pathways(hops, &edges, window_s);
    StreamMap { enabled: !endpoints.is_empty(), nodes: nodes.into_values().collect(), edges, pathways }
}

/// Most pathways listed, and the longest one followed.
const MAX_PATHWAYS: usize = 200;
const MAX_HOPS: usize = 12;

/// Whole-range hops merged per (producer, system, topic, consumer).
fn merged_hops(hops: &[HopRow]) -> BTreeMap<HopKey, Latency> {
    let mut out: BTreeMap<HopKey, Latency> = BTreeMap::new();
    for h in hops {
        out.entry(hop_key(h)).or_default().add(h);
    }
    out
}

/// End-to-end pathways: chains of hops from services that consume nothing linked (or every
/// producer, when all of them consume too) to services whose consumed messages lead nowhere.
pub(crate) fn pathways(hops: &[HopRow], edges: &[StreamEdge], window_s: f64) -> Vec<StreamPathway> {
    let merged = merged_hops(hops);
    let mut out_of: BTreeMap<&str, Vec<(&HopKey, &Latency)>> = BTreeMap::new();
    for (k, l) in &merged {
        out_of.entry(k.0.as_str()).or_default().push((k, l));
    }
    let consumers: BTreeSet<&str> = merged.keys().map(|k| k.3.as_str()).collect();
    let mut starts: Vec<&str> = out_of.keys().copied().filter(|p| !consumers.contains(p)).collect();
    if starts.is_empty() {
        starts = out_of.keys().copied().collect();
    }
    let lagging: BTreeSet<(&str, &str)> = edges.iter().filter(|e| e.lag_high).map(|e| (e.source.as_str(), e.target.as_str())).collect();
    let mut found: Vec<StreamPathway> = Vec::new();
    let mut seen = BTreeSet::new();
    // Depth-first, the path so far as hops.
    type Hop<'a> = (&'a HopKey, &'a Latency);
    fn walk<'a>(at: &str, path: &mut Vec<Hop<'a>>, out_of: &BTreeMap<&str, Vec<Hop<'a>>>, emit: &mut dyn FnMut(&[Hop<'a>])) {
        let next: Vec<_> = out_of.get(at).map(|v| v.iter().filter(|(k, _)| !path.iter().any(|(p, _)| p.0 == k.3 || p.3 == k.3)).copied().collect()).unwrap_or_default();
        if next.is_empty() || path.len() >= MAX_HOPS {
            if !path.is_empty() {
                emit(path);
            }
            return;
        }
        for hop in next {
            path.push(hop);
            walk(&hop.0 .3, path, out_of, emit);
            path.pop();
        }
    }
    for start in starts {
        let mut path = Vec::new();
        walk(start, &mut path, &out_of, &mut |hops| {
            let mut nodes = vec![service_id(&hops[0].0 .0)];
            let mut lag_high = false;
            for (k, _) in hops {
                let topic = topic_id(&k.1, &k.2);
                let consumer = service_id(&k.3);
                lag_high |= lagging.contains(&(topic.as_str(), consumer.as_str()));
                nodes.push(topic);
                nodes.push(consumer);
            }
            let id = nodes.join(">");
            if seen.insert(id.clone()) {
                found.push(StreamPathway {
                    id,
                    nodes,
                    hops: hops.len() as u64,
                    latency_p50_ms: hops.iter().filter_map(|(_, l)| l.p50()).sum(),
                    latency_p99_ms: hops.iter().map(|(_, l)| l.p99).sum(),
                    throughput: hops.last().map_or(0.0, |(_, l)| l.n as f64 / window_s.max(1.0)),
                    lag_high,
                });
            }
        });
    }
    found.sort_by(|a, b| b.throughput.total_cmp(&a.throughput).then(a.id.cmp(&b.id)));
    found.truncate(MAX_PATHWAYS);
    found
}

/// The fewest hops (producer, system, topic, consumer) that carry messages from service `start` to `end`.
pub(crate) fn path_between(hops: &[HopRow], start: &str, end: &str) -> Option<Vec<HopKey>> {
    let merged = merged_hops(hops);
    let mut prev: HashMap<String, HopKey> = HashMap::new();
    let mut queue = VecDeque::from([start.to_string()]);
    let mut visited = BTreeSet::from([start.to_string()]);
    while let Some(at) = queue.pop_front() {
        if at == end {
            let mut path = Vec::new();
            let mut cur = end.to_string();
            while cur != start {
                let k = prev.get(&cur)?.clone();
                cur = k.0.clone();
                path.push(k);
            }
            path.reverse();
            return (!path.is_empty()).then_some(path);
        }
        for k in merged.keys().filter(|k| k.0 == at) {
            if visited.insert(k.3.clone()) {
                prev.insert(k.3.clone(), k.clone());
                queue.push_back(k.3.clone());
            }
        }
    }
    None
}

/// Latency along `path` per interval: the hops' p50s and p99s added up, where every hop has messages.
pub(crate) fn path_latency(hops: &[HopRow], path: &[HopKey]) -> Vec<StreamLatencyPoint> {
    let mut per_t: BTreeMap<i64, HashMap<usize, Latency>> = BTreeMap::new();
    for h in hops {
        let k = hop_key(h);
        if let Some(i) = path.iter().position(|p| *p == k) {
            per_t.entry(h.t).or_default().entry(i).or_default().add(h);
        }
    }
    per_t
        .into_iter()
        .filter(|(_, by_hop)| by_hop.len() == path.len())
        .map(|(t, by_hop)| StreamLatencyPoint { t, p50_ms: by_hop.values().filter_map(Latency::p50).sum(), p99_ms: by_hop.values().map(|l| l.p99).sum() })
        .collect()
}

/// Lag per consumer group and topic over time (messages behind, all partitions), for `topics`
/// and, when given, only `groups`.
pub(crate) fn lag_series(values: &[super::metric_inventory::SeriesValue], topics: &BTreeSet<String>, groups: Option<&BTreeSet<String>>) -> Vec<StreamLagSeries> {
    let mut acc: BTreeMap<(String, String), BTreeMap<i64, f64>> = BTreeMap::new();
    for s in group_series(values) {
        if s.metric() == kafka_console::names::LAG_SECONDS {
            continue;
        }
        let (Some(group), Some(topic)) = (s.tag(&["consumer_group", "group"]), s.tag(&["topic"])) else { continue };
        if !topics.contains(&topic) || groups.is_some_and(|g| !g.contains(&group)) {
            continue;
        }
        let points = acc.entry((group, topic)).or_default();
        for (t, v) in &s.points {
            *points.entry(*t).or_default() += v.max(0.0);
        }
    }
    acc.into_iter().map(|((group, topic), points)| StreamLagSeries { group, topic, points: points.into_iter().collect() }).collect()
}

// ================================================================ handlers

#[derive(Deserialize, ToSchema)]
pub struct StreamMapRequest {
    pub range: TimeRange,
    /// `deployment.environment`; all when empty.
    #[serde(default)]
    pub env: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct StreamMeasureRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub env: Option<String>,
    /// Node id of the service messages leave (`service:<name>`).
    pub start: String,
    /// Node id of the service they reach.
    pub end: String,
}

#[derive(Deserialize, ToSchema)]
pub struct StreamNodeRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub env: Option<String>,
    /// `service:<name>` or `topic:<system>:<name>`.
    pub node: String,
}

fn env_of(env: &Option<String>) -> Option<&str> {
    env.as_deref().filter(|e| !e.is_empty() && *e != "*")
}

fn bound(q: clickhouse::query::Query, range: &TimeRange, env: Option<&str>) -> clickhouse::query::Query {
    let q = q.bind(range.from).bind(range.to);
    match env {
        Some(e) => q.bind(e),
        None => q,
    }
}

async fn fetch_endpoints(st: &AppState, org: &str, range: &TimeRange, env: Option<&str>) -> Result<Vec<EndpointRow>, ApiError> {
    Ok(bound(st.ch(org).query(&endpoints_sql(env.is_some())), range, env).fetch_all::<EndpointRow>().await?)
}

async fn fetch_hops(st: &AppState, org: &str, range: &TimeRange, env: Option<&str>, step: Option<i64>) -> Result<Vec<HopRow>, ApiError> {
    Ok(bound(st.ch(org).query(&hops_sql(env.is_some(), step)), range, env).fetch_all::<HopRow>().await?)
}

/// Consumer groups and their current lag on each topic (the range's last interval), from the Kafka metrics.
async fn consumer_lags(st: &AppState, org: &str, range: &TimeRange) -> Result<Vec<KafkaConsumerGroup>, ApiError> {
    let step = chart_step(range.to - range.from, 60);
    let values = series_values(st, org, LAG_METRICS, range.from, range.to, Some(step)).await?;
    Ok(kafka_console::aggregate(&values).consumer_groups)
}

/// Services and the topics between them, with throughput, latency, lag and end-to-end pathways.
#[utoipa::path(post, path = "/api/v1/data-streams/map", operation_id = "data_streams_map", request_body = StreamMapRequest, responses((status = 200, body = StreamMap)), tag = "data-streams")]
pub async fn map(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<StreamMapRequest>) -> ApiResult<StreamMap> {
    req.range.validate()?;
    let env = env_of(&req.env);
    let (endpoints, hops, lags) = tokio::try_join!(fetch_endpoints(&st, &org, &req.range, env), fetch_hops(&st, &org, &req.range, env, None), consumer_lags(&st, &org, &req.range))?;
    Ok(Json(build_map(&endpoints, &hops, &lags, (req.range.to - req.range.from) as f64 / 1000.0)))
}

fn service_name(id: &str) -> Result<&str, ApiError> {
    id.strip_prefix("service:").filter(|s| !s.is_empty()).ok_or_else(|| ApiError::BadRequest(format!("{id} is not a service (service:<name>)")))
}

/// Latency between two services over time, along the hops that carry messages from one to the other.
#[utoipa::path(post, path = "/api/v1/data-streams/measure", operation_id = "data_streams_measure", request_body = StreamMeasureRequest, responses((status = 200, body = StreamMeasure)), tag = "data-streams")]
pub async fn measure(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<StreamMeasureRequest>) -> ApiResult<StreamMeasure> {
    req.range.validate()?;
    let (start, end) = (service_name(&req.start)?, service_name(&req.end)?);
    if start == end {
        return Err(ApiError::BadRequest("start and end must differ".into()));
    }
    let env = env_of(&req.env);
    let step = chart_step(req.range.to - req.range.from, 60);
    let (whole, bucketed) = tokio::try_join!(fetch_hops(&st, &org, &req.range, env, None), fetch_hops(&st, &org, &req.range, env, Some(step)))?;
    let Some(path) = path_between(&whole, start, end) else { return Ok(Json(StreamMeasure::default())) };
    let merged = merged_hops(&whole);
    let hop_latency: Vec<&Latency> = path.iter().filter_map(|k| merged.get(k)).collect();
    let mut nodes = vec![service_id(start)];
    for k in &path {
        nodes.push(topic_id(&k.1, &k.2));
        nodes.push(service_id(&k.3));
    }
    let window_s = ((req.range.to - req.range.from) as f64 / 1000.0).max(1.0);
    Ok(Json(StreamMeasure {
        path: nodes,
        latency_p50_ms: Some(hop_latency.iter().filter_map(|l| l.p50()).sum()),
        latency_p99_ms: Some(hop_latency.iter().map(|l| l.p99).sum()),
        throughput: hop_latency.last().map(|l| l.n as f64 / window_s),
        series: path_latency(&bucketed, &path),
    }))
}

/// One node's messages over time and the lag of the consumer groups behind it.
#[utoipa::path(post, path = "/api/v1/data-streams/node", operation_id = "data_streams_node", request_body = StreamNodeRequest, responses((status = 200, body = StreamNodeDetail)), tag = "data-streams")]
pub async fn node(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<StreamNodeRequest>) -> ApiResult<StreamNodeDetail> {
    req.range.validate()?;
    let env = env_of(&req.env);
    let step = chart_step(req.range.to - req.range.from, 60);
    let per_s = |points: Vec<(i64, u64)>| points.into_iter().map(|(t, n)| (t, n as f64 * 1000.0 / step as f64)).collect::<Vec<_>>();
    #[derive(Row, Deserialize)]
    struct RateRow {
        t: i64,
        produced: u64,
        consumed: u64,
    }
    let (rows, topics, groups): (Vec<RateRow>, BTreeSet<String>, Option<BTreeSet<String>>) = if let Some(rest) = req.node.strip_prefix("topic:") {
        let (system, topic) = rest.split_once(':').ok_or_else(|| ApiError::BadRequest("topic nodes are topic:<system>:<name>".into()))?;
        let q = bound(st.ch(&org).query(&node_rate_sql(env.is_some(), step, true)), &req.range, env).bind(system).bind(topic);
        (q.fetch_all().await?, BTreeSet::from([topic.to_string()]), None)
    } else {
        let service = service_name(&req.node)?;
        let q = bound(st.ch(&org).query(&node_rate_sql(env.is_some(), step, false)), &req.range, env).bind(service);
        let endpoints = fetch_endpoints(&st, &org, &req.range, env).await?;
        let consumed: Vec<&EndpointRow> = endpoints.iter().filter(|e| e.service == service && e.role == "consume").collect();
        (q.fetch_all().await?, consumed.iter().map(|e| e.topic.clone()).collect(), Some(consumed.iter().filter(|e| !e.grp.is_empty()).map(|e| e.grp.clone()).collect()))
    };
    let lag = if topics.is_empty() {
        Vec::new()
    } else {
        let values = series_values(&st, &org, LAG_METRICS, req.range.from, req.range.to, Some(step)).await?;
        lag_series(&values, &topics, groups.as_ref())
    };
    Ok(Json(StreamNodeDetail { produced: per_s(rows.iter().map(|r| (r.t, r.produced)).collect()), consumed: per_s(rows.iter().map(|r| (r.t, r.consumed)).collect()), lag }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::kafka_console::KafkaGroupTopic;
    use crate::routes::metric_inventory::SeriesValue;

    fn ep(service: &str, topic: &str, role: &str, grp: &str, spans: u64) -> EndpointRow {
        EndpointRow { service: service.into(), system: "kafka".into(), topic: topic.into(), role: role.into(), grp: grp.into(), spans, errors: 0, p50_ms: 2.0, p99_ms: 9.0 }
    }

    fn hop(producer: &str, topic: &str, consumer: &str, t: i64, n: u64, p50: f64, p99: f64) -> HopRow {
        HopRow { producer: producer.into(), consumer: consumer.into(), system: "kafka".into(), topic: topic.into(), t, n, p50_ms: p50, p99_ms: p99 }
    }

    fn group(name: &str, topic: &str, lag: f64, secs: Option<f64>) -> KafkaConsumerGroup {
        KafkaConsumerGroup { name: name.into(), cluster: "shop-events".into(), topics: vec![KafkaGroupTopic { topic: topic.into(), lag, lag_seconds: secs }], ..Default::default() }
    }

    /// order-service → orders.created → billing → payments.authorized → order-service is a cycle;
    /// storefront-api → orders.created too; inventory-service consumes orders.created.
    fn shop() -> (Vec<EndpointRow>, Vec<HopRow>) {
        let endpoints = vec![
            ep("storefront-api", "orders.created", "produce", "", 600),
            ep("billing", "orders.created", "consume", "billing-consumer", 600),
            ep("inventory-service", "orders.created", "consume", "inventory-consumer", 600),
            ep("billing", "payments.authorized", "produce", "", 300),
            ep("order-service", "payments.authorized", "consume", "order-consumer", 300),
        ];
        let hops = vec![
            hop("storefront-api", "orders.created", "billing", 0, 600, 12.0, 40.0),
            hop("storefront-api", "orders.created", "inventory-service", 0, 600, 20.0, 90.0),
            hop("billing", "payments.authorized", "order-service", 0, 300, 5.0, 15.0),
        ];
        (endpoints, hops)
    }

    #[test]
    fn map_has_services_topics_and_both_edge_kinds() {
        let (endpoints, hops) = shop();
        let m = build_map(&endpoints, &hops, &[group("billing-consumer", "orders.created", 50_000.0, Some(120.0)), group("inventory-consumer", "orders.created", 10.0, None)], 60.0);
        assert!(m.enabled);
        let ids: Vec<&str> = m.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["service:billing", "service:inventory-service", "service:order-service", "service:storefront-api", "topic:kafka:orders.created", "topic:kafka:payments.authorized"]);
        let topic = m.nodes.iter().find(|n| n.id == "topic:kafka:orders.created").unwrap();
        assert_eq!((topic.produced, topic.consumed), (10.0, 20.0), "10 msg/s in, delivered to two consumers");
        assert_eq!((topic.lag, topic.lag_seconds, topic.lag_high), (Some(50_000.0), Some(120.0), true));
        let to_billing = m.edges.iter().find(|e| e.target == "service:billing").unwrap();
        assert_eq!(to_billing.kind, StreamEdgeKind::Consume);
        assert_eq!((to_billing.latency_p50_ms, to_billing.latency_p99_ms), (Some(12.0), Some(40.0)));
        assert_eq!(to_billing.consumer_groups, vec!["billing-consumer".to_string()]);
        assert!(to_billing.lag_high);
        let to_inventory = m.edges.iter().find(|e| e.target == "service:inventory-service").unwrap();
        assert_eq!((to_inventory.lag, to_inventory.lag_high), (Some(10.0), false), "lag of its own group only");
        let publish = m.edges.iter().find(|e| e.source == "service:storefront-api").unwrap();
        assert_eq!((publish.kind, publish.throughput, publish.latency_p50_ms), (StreamEdgeKind::Produce, 10.0, None));
        let billing = m.nodes.iter().find(|n| n.id == "service:billing").unwrap();
        assert_eq!((billing.produced, billing.consumed, billing.lag_high), (5.0, 10.0, true));
    }

    #[test]
    fn pathways_follow_hops_from_the_first_producer() {
        let (endpoints, hops) = shop();
        let m = build_map(&endpoints, &hops, &[group("billing-consumer", "orders.created", 50_000.0, Some(120.0))], 60.0);
        let ids: Vec<&str> = m.pathways.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "service:storefront-api>topic:kafka:orders.created>service:inventory-service",
                "service:storefront-api>topic:kafka:orders.created>service:billing>topic:kafka:payments.authorized>service:order-service",
            ],
            "by throughput at the end"
        );
        let long = &m.pathways[1];
        assert_eq!((long.hops, long.latency_p50_ms, long.latency_p99_ms, long.throughput), (2, 17.0, 55.0, 5.0));
        assert!(long.lag_high, "billing lags on the way");
        assert!(!m.pathways[0].lag_high);
    }

    #[test]
    fn cycles_end_a_pathway() {
        let hops = vec![hop("order-service", "orders.created", "billing", 0, 10, 1.0, 2.0), hop("billing", "payments.authorized", "order-service", 0, 10, 1.0, 2.0)];
        let p = pathways(&hops, &[], 10.0);
        assert_eq!(p.len(), 2, "every producer consumes: start from each");
        assert!(p.iter().all(|p| p.hops == 1), "a pathway stops before it reaches a service twice");
    }

    #[test]
    fn measure_finds_the_shortest_path_and_adds_latency_per_interval() {
        let (_, hops) = shop();
        let path = path_between(&hops, "storefront-api", "order-service").unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].3, "billing");
        assert!(path_between(&hops, "order-service", "storefront-api").is_none(), "messages don't flow back");
        let bucketed = vec![
            hop("storefront-api", "orders.created", "billing", 0, 5, 10.0, 30.0),
            hop("billing", "payments.authorized", "order-service", 0, 5, 4.0, 8.0),
            hop("storefront-api", "orders.created", "billing", 60_000, 5, 12.0, 50.0),
            // Interval with only the first hop: no end-to-end point.
            hop("storefront-api", "orders.created", "billing", 120_000, 5, 12.0, 50.0),
            hop("billing", "payments.authorized", "order-service", 60_000, 5, 6.0, 9.0),
            hop("storefront-api", "orders.created", "inventory-service", 60_000, 5, 99.0, 99.0),
        ];
        let series = path_latency(&bucketed, &path);
        assert_eq!(series, vec![StreamLatencyPoint { t: 0, p50_ms: 14.0, p99_ms: 38.0 }, StreamLatencyPoint { t: 60_000, p50_ms: 18.0, p99_ms: 59.0 }]);
    }

    #[test]
    fn lag_series_sum_partitions_per_group() {
        let sv = |t: i64, v: f64, group: &str, topic: &str, partition: &str| SeriesValue {
            metric: "kafka.consumer_lag".into(),
            res: HashMap::new(),
            attrs: [("consumer_group", group), ("topic", topic), ("partition", partition)].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            t,
            value: v,
            max: v,
            count: 1,
            last: t,
        };
        let values = vec![
            sv(0, 5.0, "billing-consumer", "orders.created", "0"),
            sv(0, 7.0, "billing-consumer", "orders.created", "1"),
            sv(60_000, 1.0, "billing-consumer", "orders.created", "0"),
            sv(0, 3.0, "audit-consumer", "orders.created", "0"),
            sv(0, 9.0, "billing-consumer", "inventory.reserved", "0"),
        ];
        let topics = BTreeSet::from(["orders.created".to_string()]);
        let all = lag_series(&values, &topics, None);
        assert_eq!(all.len(), 2);
        let billing = all.iter().find(|s| s.group == "billing-consumer").unwrap();
        assert_eq!(billing.points, vec![(0, 12.0), (60_000, 1.0)]);
        let only = lag_series(&values, &topics, Some(&BTreeSet::from(["audit-consumer".to_string()])));
        assert_eq!(only.len(), 1);
    }

    #[test]
    fn queries_filter_by_org_and_bind_input() {
        for sql in [endpoints_sql(true), hops_sql(false, Some(60_000)), hops_sql(true, None), node_rate_sql(true, 60_000, true), node_rate_sql(false, 60_000, false)] {
            assert!(sql.contains("OrgId = {oc_org:String}"));
            assert!(sql.contains("fromUnixTimestamp64Milli(?)"));
        }
        assert_eq!(endpoints_sql(true).matches('?').count(), 3);
        assert_eq!(node_rate_sql(false, 60_000, true).matches('?').count(), 4);
        assert!(service_name("topic:kafka:x").is_err());
        assert_eq!(service_name("service:billing").unwrap(), "billing");
    }
}
