//! Synthetic Kafka metrics, as the Datadog Agent's `kafka` (broker JMX) and `kafka_consumer` checks
//! send them (`/api/v2/series`, names from `integrations-core/kafka/metadata.csv` and
//! `integrations-core/kafka_consumer/metadata.csv`, BSD-3-Clause, Copyright (c) Datadog, Inc.), so the
//! Data Streams Kafka Console has clusters, topics, brokers, consumer groups and connectors.
//!
//! Two fictional clusters: `shop-events` (three brokers, the topics the demo services publish and
//! consume in their traces) and `analytics-events` (one broker, a clickstream topic). Every value is a
//! function of time, so restarts and the backfill agree: offsets grow with each topic's rate and
//! `inventory-consumer` falls behind for five minutes out of every twenty (as its spans do).

use serde_json::{json, Value};

/// Five minutes out of every twenty, `inventory-service` consumes slowly and its lag builds up.
pub fn inventory_lagging(now_s: u64) -> bool {
    now_s % 1200 >= 900
}

struct Topic {
    cluster: &'static str,
    name: &'static str,
    partitions: u32,
    /// Messages per second, all partitions.
    rate: f64,
    bytes_per_message: f64,
    /// Consumer groups and their usual lag (messages per partition).
    groups: &'static [(&'static str, f64)],
}

const TOPICS: &[Topic] = &[
    Topic { cluster: "shop-events", name: "orders.created", partitions: 6, rate: 42.0, bytes_per_message: 1_350.0, groups: &[("billing-consumer", 4.0), ("inventory-consumer", 6.0)] },
    Topic { cluster: "shop-events", name: "payments.authorized", partitions: 3, rate: 40.0, bytes_per_message: 820.0, groups: &[("notification-consumer", 2.0)] },
    Topic { cluster: "shop-events", name: "inventory.reserved", partitions: 3, rate: 41.0, bytes_per_message: 640.0, groups: &[("catalog-consumer", 3.0)] },
    Topic { cluster: "shop-events", name: "order-events", partitions: 3, rate: 18.0, bytes_per_message: 900.0, groups: &[("notification-consumer", 1.0)] },
    Topic { cluster: "analytics-events", name: "clickstream.events", partitions: 12, rate: 260.0, bytes_per_message: 410.0, groups: &[("analytics-loader", 120.0)] },
];

/// cluster → brokers (id, host); the first one is the controller.
const BROKERS: &[(&str, &[(u32, &str)])] =
    &[("shop-events", &[(1, "i-0a1b2c3d4e5f60011"), (2, "i-0a1b2c3d4e5f60012"), (3, "i-0a1b2c3d4e5f60013")]), ("analytics-events", &[(1, "i-0a1b2c3d4e5f60021")])];

/// Kafka Connect connectors of `shop-events`: name, type, class, tasks.
const CONNECTORS: &[(&str, &str, &str, u32)] = &[("orders-s3-sink", "sink", "S3SinkConnector", 2), ("catalog-cdc-source", "source", "PostgresConnector", 1)];

/// A reference time (2025-01-01) the offsets grow from.
const EPOCH_S: f64 = 1_735_689_600.0;

fn wave(now_s: u64, period: f64, phase: f64) -> f64 {
    1.0 + 0.15 * ((now_s as f64 / period) + phase).sin()
}

struct Out {
    series: Vec<Value>,
    ts: i64,
}

impl Out {
    fn gauge(&mut self, metric: &str, value: f64, host: &str, tags: &[String]) {
        self.series.push(json!({
            "metric": metric,
            "type": 3,
            "points": [{ "timestamp": self.ts, "value": value }],
            "resources": [{ "type": "host", "name": host }],
            "tags": tags,
        }));
    }
}

fn tag(k: &str, v: impl std::fmt::Display) -> String {
    format!("{k}:{v}")
}

/// The Agent's series at `now_s` (unix seconds) for both clusters.
pub fn series_payload(now_s: u64) -> Value {
    let mut out = Out { series: Vec::new(), ts: now_s as i64 };
    let elapsed = now_s as f64 - EPOCH_S;
    let lagging = inventory_lagging(now_s);
    for (cluster, nodes) in BROKERS {
        let check_host = nodes[0].1;
        let cid = tag("kafka_cluster_id", cluster);
        let jmx = tag("kafka_cluster", cluster);
        let topics: Vec<&Topic> = TOPICS.iter().filter(|t| t.cluster == *cluster).collect();
        let total_partitions: u32 = topics.iter().map(|t| t.partitions).sum();
        // One broker of shop-events loses a replica for a few minutes every half hour.
        let degraded = *cluster == "shop-events" && now_s % 1800 < 240;
        // Broker JMX metrics (the kafka check runs on every broker).
        for (i, (id, host)) in nodes.iter().enumerate() {
            let share = 1.0 / nodes.len() as f64;
            let msgs: f64 = topics.iter().map(|t| t.rate * wave(now_s, 300.0, i as f64)).sum::<f64>() * share;
            let bytes: f64 = topics.iter().map(|t| t.rate * t.bytes_per_message).sum::<f64>() * share * wave(now_s, 300.0, i as f64);
            let tags = [jmx.clone(), tag("env", "prod")];
            out.gauge("kafka.messages_in.rate", msgs, host, &tags);
            out.gauge("kafka.net.bytes_in.rate", bytes, host, &tags);
            out.gauge("kafka.net.bytes_out.rate", bytes * 2.1, host, &tags);
            out.gauge("kafka.replication.under_replicated_partitions", if degraded && *id == 3 { 2.0 } else { 0.0 }, host, &tags);
            out.gauge("kafka.replication.offline_partitions_count", 0.0, host, &tags);
            out.gauge("kafka.replication.active_controller_count", if i == 0 { 1.0 } else { 0.0 }, host, &tags);
            let replicas = (total_partitions * nodes.len().min(3) as u32) as f64 * share;
            out.gauge("kafka.replication.partition_count", replicas.round(), host, &tags);
            out.gauge("kafka.replication.leader_count", (total_partitions as f64 * share).round(), host, &tags);
            for t in &topics {
                let topic_tags = [jmx.clone(), tag("env", "prod"), tag("topic", t.name)];
                out.gauge("kafka.topic.messages_in.rate", t.rate * share * wave(now_s, 300.0, i as f64), host, &topic_tags);
                out.gauge("kafka.topic.net.bytes_in.rate", t.rate * t.bytes_per_message * share, host, &topic_tags);
                out.gauge("kafka.topic.net.bytes_out.rate", t.rate * t.bytes_per_message * share * t.groups.len() as f64, host, &topic_tags);
            }
        }
        // The kafka_consumer check: cluster metadata, offsets and lag.
        out.gauge("kafka.broker.count", nodes.len() as f64, check_host, std::slice::from_ref(&cid));
        out.gauge("kafka.cluster.controller_id", nodes[0].0 as f64, check_host, std::slice::from_ref(&cid));
        out.gauge("kafka.topic.count", topics.len() as f64, check_host, std::slice::from_ref(&cid));
        for (id, host) in nodes.iter() {
            let tags = [cid.clone(), tag("broker_id", id), tag("broker_host", format!("{host}.ec2.internal"))];
            out.gauge("kafka.broker.partition_count", (total_partitions * nodes.len().min(3) as u32 / nodes.len() as u32) as f64, check_host, &tags);
            out.gauge("kafka.broker.leader_count", (total_partitions / nodes.len() as u32) as f64, check_host, &tags);
        }
        let replication = nodes.len().min(3) as f64;
        for t in &topics {
            let topic_tags = [cid.clone(), tag("topic", t.name)];
            out.gauge("kafka.topic.partitions", t.partitions as f64, check_host, &topic_tags);
            out.gauge("kafka.topic.message_rate", t.rate, check_host, &topic_tags);
            let per_partition = t.rate / t.partitions as f64;
            let mut size = 0.0;
            for p in 0..t.partitions {
                let part = [cid.clone(), tag("topic", t.name), tag("partition", p)];
                // Partitions differ a little in traffic.
                let offset = (elapsed * per_partition * (1.0 + 0.05 * p as f64)).floor();
                size += offset.min(per_partition * 7.0 * 86_400.0);
                out.gauge("kafka.broker_offset", offset, check_host, &part);
                let under = degraded && p == 0;
                let mut repl = part.to_vec();
                repl.push(tag("leader_broker_id", 1 + p % nodes.len() as u32));
                out.gauge("kafka.partition.replicas", replication, check_host, &repl);
                out.gauge("kafka.partition.isr", if under { replication - 1.0 } else { replication }, check_host, &repl);
                out.gauge("kafka.partition.under_replicated", if under { 1.0 } else { 0.0 }, check_host, &repl);
                out.gauge("kafka.partition.offline", 0.0, check_host, &repl);
                for (group, usual) in t.groups {
                    let behind = if *group == "inventory-consumer" && lagging { usual + per_partition * ((now_s % 1200) - 900) as f64 * 0.9 } else { usual * wave(now_s, 60.0, p as f64) };
                    let behind = behind.round();
                    let g = [cid.clone(), tag("topic", t.name), tag("partition", p), tag("consumer_group", group), tag("consumer_group_state", "stable")];
                    out.gauge("kafka.consumer_offset", offset - behind, check_host, &g);
                    out.gauge("kafka.consumer_lag", behind, check_host, &g);
                    out.gauge("kafka.estimated_consumer_lag", behind / per_partition.max(0.001), check_host, &g);
                }
            }
            out.gauge("kafka.topic.size", size, check_host, &topic_tags);
        }
        let groups: std::collections::BTreeSet<&str> = topics.iter().flat_map(|t| t.groups.iter().map(|g| g.0)).collect();
        out.gauge("kafka.consumer_group.count", groups.len() as f64, check_host, std::slice::from_ref(&cid));
        for g in groups {
            let members = match g {
                "analytics-loader" => 6.0,
                "billing-consumer" => 3.0,
                _ => 2.0,
            };
            out.gauge("kafka.consumer_group.members", members, check_host, &[cid.clone(), tag("consumer_group", g), tag("consumer_group_state", "stable")]);
        }
        if *cluster == "shop-events" {
            out.gauge("kafka.connector.count", CONNECTORS.len() as f64, check_host, std::slice::from_ref(&cid));
            // The CDC source's task fails for ten minutes every hour.
            let cdc_failed = now_s % 3600 < 600;
            for (name, kind, class, tasks) in CONNECTORS {
                let failed = if *name == "catalog-cdc-source" && cdc_failed { 1 } else { 0 };
                let state = if failed > 0 { "failed" } else { "running" };
                let base = [cid.clone(), tag("connector", name), tag("connector_type", kind), tag("connector_class", class), tag("connector_state", state)];
                out.gauge("kafka.connector.task.count", *tasks as f64, check_host, &base);
                for (task_state, n) in [("running", tasks - failed), ("failed", failed)] {
                    if n > 0 {
                        let mut t = base.to_vec();
                        t.push(tag("task_state", task_state));
                        out.gauge("kafka.connector.tasks", n as f64, check_host, &t);
                    }
                }
            }
        }
    }
    json!({ "series": out.series })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(payload: &Value, metric: &str) -> Vec<(Vec<String>, f64)> {
        payload["series"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["metric"] == metric)
            .map(|s| (s["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect(), s["points"][0]["value"].as_f64().unwrap()))
            .collect()
    }

    #[test]
    fn consumers_trail_the_broker_and_offsets_grow() {
        // 1_800_000_000 is a multiple of 1200: the start of a twenty-minute cycle.
        let now = 1_800_000_000 + 60;
        let a = series_payload(now);
        let b = series_payload(now + 60);
        let offsets = |p: &Value| values(p, "kafka.broker_offset").into_iter().find(|(t, _)| t.contains(&"topic:orders.created".to_string()) && t.contains(&"partition:0".to_string())).unwrap().1;
        let rate = (offsets(&b) - offsets(&a)) / 60.0;
        assert!((rate - 7.0).abs() < 0.1, "42 messages/s over 6 partitions: {rate}");
        for (tags, lag) in values(&a, "kafka.consumer_lag") {
            assert!(lag >= 0.0, "{tags:?}");
        }
        assert!(values(&a, "kafka.consumer_lag").iter().all(|(_, lag)| *lag < 200.0), "nobody lags outside the window");
    }

    #[test]
    fn inventory_consumer_lags_five_minutes_in_twenty() {
        let start = 1_800_000_000;
        assert!(!inventory_lagging(start + 899));
        assert!(inventory_lagging(start + 900));
        let p = series_payload(start + 1100);
        let worst = values(&p, "kafka.estimated_consumer_lag").into_iter().filter(|(t, _)| t.contains(&"consumer_group:inventory-consumer".to_string())).map(|x| x.1).fold(0.0, f64::max);
        assert!(worst > 60.0, "over a minute behind: {worst}");
    }

    #[test]
    fn clusters_are_fictional_and_tagged() {
        let p = series_payload(1_800_000_000);
        for s in p["series"].as_array().unwrap() {
            let tags: Vec<&str> = s["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
            assert!(tags.iter().any(|t| t.starts_with("kafka_cluster")), "{}", s["metric"]);
            assert!(s["resources"][0]["name"].as_str().unwrap().starts_with("i-0a1b2c3d4e5f600"));
        }
        assert_eq!(values(&p, "kafka.broker.count").iter().map(|x| x.1).sum::<f64>(), 4.0);
    }
}
