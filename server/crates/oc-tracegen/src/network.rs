//! Synthetic network connections, every 30 seconds, in the shape the intake writes for the Datadog
//! Agent's system-probe (`/api/v1/connections` → OTLP log records with resource
//! `oc.signal = dd.connections`), so Cloud Network Monitoring has traffic between the demo
//! services. Each monitored client reports its outgoing connections with its own tags; the server
//! reports the same connection as incoming. Databases have no Agent (their side resolves to N/A),
//! and a payment provider and a mail relay are external.

use serde_json::{json, Value};

use crate::{attrs_json, Rng};

/// A monitored workload: its address, host and tags.
struct Node {
    service: &'static str,
    ip: &'static str,
    host: &'static str,
    zone: &'static str,
    namespace: &'static str,
}

const NODES: &[Node] = &[
    Node { service: "api-gateway", ip: "10.0.1.11", host: "i-0a1b2c3d4e5f60001", zone: "us-east-1a", namespace: "edge" },
    Node { service: "storefront-api", ip: "10.0.1.12", host: "i-0a1b2c3d4e5f60002", zone: "us-east-1b", namespace: "shop" },
    Node { service: "order-service", ip: "10.0.1.13", host: "i-0a1b2c3d4e5f60001", zone: "us-east-1a", namespace: "shop" },
    Node { service: "billing", ip: "10.0.1.14", host: "i-0a1b2c3d4e5f60002", zone: "us-east-1b", namespace: "payments" },
    Node { service: "inventory-service", ip: "10.0.2.15", host: "i-0a1b2c3d4e5f60003", zone: "us-east-1a", namespace: "shop" },
    Node { service: "catalog-service", ip: "10.0.2.16", host: "i-0a1b2c3d4e5f60004", zone: "us-east-1c", namespace: "shop" },
    Node { service: "workflow-engine", ip: "10.0.2.17", host: "i-0a1b2c3d4e5f60004", zone: "us-east-1c", namespace: "payments" },
    Node { service: "notification-worker", ip: "10.0.3.18", host: "i-0a1b2c3d4e5f60005", zone: "us-east-1b", namespace: "shop" },
    // The cluster's DNS server.
    Node { service: "coredns", ip: "10.0.4.10", host: "i-0a1b2c3d4e5f60003", zone: "us-east-1a", namespace: "kube-system" },
];

/// What the hosts resolved addresses from (sent by the Agent as its DNS table).
const DOMAINS: &[(&str, &str)] =
    &[("10.0.5.20", "postgres.internal.example.net"), ("10.0.5.21", "redis.internal.example.net"), ("203.0.113.10", "payments.example.net"), ("198.51.100.25", "smtp.example.net")];

/// client service, domain queried, query type, requests per 30 s, NXDOMAIN share, typical
/// response time (µs).
const DNS_QUERIES: &[(&str, &str, &str, f64, f64, f64)] = &[
    ("api-gateway", "storefront-api.shop.svc.cluster.local", "A", 120.0, 0.0, 450.0),
    ("storefront-api", "catalog-service.shop.svc.cluster.local", "A", 80.0, 0.0, 420.0),
    ("storefront-api", "order-service.shop.svc.cluster.local", "A", 60.0, 0.0, 430.0),
    ("order-service", "billing.payments.svc.cluster.local", "A", 40.0, 0.0, 460.0),
    ("order-service", "inventory-service.shop.svc.cluster.local", "A", 40.0, 0.0, 440.0),
    ("order-service", "postgres.internal.example.net", "A", 12.0, 0.0, 1_900.0),
    ("catalog-service", "postgres.internal.example.net", "A", 10.0, 0.0, 1_850.0),
    ("inventory-service", "redis.internal.example.net", "A", 8.0, 0.0, 1_700.0),
    ("inventory-service", "inventory-cache.shop.svc.cluster.local", "A", 20.0, 0.08, 520.0),
    ("billing", "payments.example.net", "A", 18.0, 0.0, 18_000.0),
    ("billing", "payments.example.net", "AAAA", 18.0, 0.0, 19_000.0),
    ("workflow-engine", "legacy-erp.internal.example.net", "A", 6.0, 1.0, 2_400.0),
    ("notification-worker", "smtp.example.net", "A", 9.0, 0.0, 21_000.0),
    ("notification-worker", "smtp.example.net", "MX", 3.0, 0.0, 23_000.0),
];

/// client service → server (a service, or an address without an Agent), port, bytes sent per
/// 30 s, bytes received per 30 s, connections opened per 30 s, typical RTT (µs).
const EDGES: &[(&str, &str, u32, f64, f64, u32, f64)] = &[
    ("api-gateway", "storefront-api", 8080, 2.4e6, 9.6e6, 40, 380.0),
    ("storefront-api", "catalog-service", 8080, 0.9e6, 6.1e6, 22, 920.0),
    ("storefront-api", "order-service", 8080, 1.2e6, 0.8e6, 12, 410.0),
    ("order-service", "billing", 8080, 0.4e6, 0.2e6, 8, 450.0),
    ("order-service", "inventory-service", 8080, 0.3e6, 0.5e6, 9, 390.0),
    ("order-service", "workflow-engine", 8080, 0.2e6, 0.1e6, 4, 1100.0),
    ("workflow-engine", "notification-worker", 8080, 0.1e6, 0.05e6, 3, 1050.0),
    ("order-service", "10.0.5.20", 5432, 1.6e6, 3.9e6, 2, 310.0),
    ("catalog-service", "10.0.5.20", 5432, 0.7e6, 5.4e6, 2, 330.0),
    ("inventory-service", "10.0.5.21", 6379, 0.5e6, 0.9e6, 1, 120.0),
    ("billing", "203.0.113.10", 443, 0.15e6, 0.3e6, 6, 24_000.0),
    ("notification-worker", "198.51.100.25", 587, 0.6e6, 0.02e6, 3, 31_000.0),
    // DNS over UDP to the cluster DNS server, which forwards to the VPC resolver.
    ("api-gateway", "coredns", 53, 9_000.0, 22_000.0, 0, 0.0),
    ("storefront-api", "coredns", 53, 11_000.0, 27_000.0, 0, 0.0),
    ("order-service", "coredns", 53, 8_000.0, 20_000.0, 0, 0.0),
    ("billing", "coredns", 53, 2_500.0, 7_000.0, 0, 0.0),
    ("inventory-service", "coredns", 53, 2_200.0, 4_500.0, 0, 0.0),
    ("catalog-service", "coredns", 53, 900.0, 2_100.0, 0, 0.0),
    ("workflow-engine", "coredns", 53, 500.0, 900.0, 0, 0.0),
    ("notification-worker", "coredns", 53, 900.0, 2_400.0, 0, 0.0),
    ("coredns", "10.0.0.2", 53, 4_000.0, 9_000.0, 0, 0.0),
];

fn node(service: &str) -> Option<&'static Node> {
    NODES.iter().find(|n| n.service == service)
}

fn tags(n: &Node) -> String {
    format!("service:{s},env:prod,availability-zone:{z},kube_namespace:{ns},pod_name:{s}-7d9f8c,container_name:{s},short_image:{s}", s = n.service, z = n.zone, ns = n.namespace)
}

/// The connections (with `connections`) and DNS stats of the 30 seconds ending at `now_ns`, as an
/// OTLP logs payload.
pub fn payload(rng: &mut Rng, now_ns: u64, connections: bool) -> Value {
    use std::collections::BTreeMap;
    let mut by_host: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for &(client, server, port, sent, received, conns, rtt) in EDGES.iter().filter(|_| connections) {
        let Some(c) = node(client) else { continue };
        let jitter = 0.7 + rng.f() * 0.6;
        let (sent, received) = ((sent * jitter) as u64, (received * jitter) as u64);
        let established = (f64::from(conns) * jitter).round() as u64;
        let retransmits = if rng.chance(0.25) { rng.range(1, 6) } else { 0 };
        let rtt_us = rtt * (0.8 + rng.f() * 0.4);
        let ephemeral = rng.range(32_768, 60_999);
        let server_ip = node(server).map(|n| n.ip).unwrap_or(server);
        let domain = DOMAINS.iter().find(|(ip, _)| *ip == server_ip).map(|(_, d)| *d).unwrap_or("");
        let record = |direction: &str, local_ip: &str, local_port: u64, ephemeral_local: bool, remote_ip: &str, remote_port: u64, out: u64, inn: u64, local_tags: String| {
            json!({
                "timeUnixNano": now_ns.to_string(),
                "body": { "stringValue": "" },
                "attributes": attrs_json(&[
                    ("direction".into(), json!(direction)),
                    ("type".into(), json!(if port == 53 { "udp" } else { "tcp" })),
                    ("family".into(), json!("v4")),
                    ("local.ip".into(), json!(local_ip)),
                    ("local.port".into(), json!(local_port.to_string())),
                    ("local.port_ephemeral".into(), json!(ephemeral_local.to_string())),
                    ("remote.ip".into(), json!(remote_ip)),
                    ("remote.port".into(), json!(remote_port.to_string())),
                    ("bytes_sent".into(), json!(out.to_string())),
                    ("bytes_received".into(), json!(inn.to_string())),
                    ("packets_sent".into(), json!((out / 1400 + 1).to_string())),
                    ("packets_received".into(), json!((inn / 1400 + 1).to_string())),
                    ("retransmits".into(), json!(retransmits.to_string())),
                    ("rtt_us".into(), json!(format!("{rtt_us:.0}"))),
                    ("rtt_var_us".into(), json!(format!("{:.0}", rtt_us * (0.1 + variance_share(rtt_us))))),
                    ("tcp_established".into(), json!(established.to_string())),
                    ("tcp_closed".into(), json!(established.to_string())),
                    ("local.tags".into(), json!(local_tags)),
                    ("remote.domain".into(), json!(if direction == "outgoing" { domain } else { "" })),
                ]),
            })
        };
        by_host.entry(c.host).or_default().push(record("outgoing", c.ip, ephemeral, true, server_ip, u64::from(port), sent, received, tags(c)));
        // The server's Agent sees the same connection coming in.
        if let Some(s) = node(server) {
            by_host.entry(s.host).or_default().push(record("incoming", s.ip, u64::from(port), false, c.ip, ephemeral, received, sent, tags(s)));
        }
    }
    // DNS stats of the queries each service made, as the Agent reports them per domain and type.
    let resolver = node("coredns").expect("coredns node");
    for &(client, domain, qtype, per_30s, nx_share, latency_us) in DNS_QUERIES {
        let Some(c) = node(client) else { continue };
        let requests = (per_30s * (0.7 + rng.f() * 0.6)).round() as u64;
        let nxdomain = (requests as f64 * nx_share).round() as u64;
        let servfail = u64::from(rng.chance(0.03));
        let timeouts = u64::from(rng.chance(0.05));
        let noerror = requests.saturating_sub(nxdomain + servfail + timeouts);
        let spread = 0.8 + rng.f() * 0.4;
        let latency = |n: u64| (n as f64 * latency_us * spread) as u64;
        let (success, failure) = (latency(noerror), latency(nxdomain + servfail));
        by_host.entry(c.host).or_default().push(json!({
            "timeUnixNano": now_ns.to_string(),
            "body": { "stringValue": "" },
            "attributes": attrs_json(&[
                ("kind".into(), json!("dns")),
                ("direction".into(), json!("outgoing")),
                ("type".into(), json!("udp")),
                ("local.ip".into(), json!(c.ip)),
                ("local.port_ephemeral".into(), json!("true")),
                ("remote.ip".into(), json!(resolver.ip)),
                ("remote.port".into(), json!("53")),
                ("local.tags".into(), json!(tags(c))),
                ("dns.query".into(), json!(domain)),
                ("dns.query_type".into(), json!(qtype)),
                ("dns.noerror".into(), json!(noerror.to_string())),
                ("dns.nxdomain".into(), json!(nxdomain.to_string())),
                ("dns.servfail".into(), json!(servfail.to_string())),
                ("dns.other_errors".into(), json!("0")),
                ("dns.timeouts".into(), json!(timeouts.to_string())),
                ("dns.success_latency_us".into(), json!(success.to_string())),
                ("dns.failure_latency_us".into(), json!(failure.to_string())),
            ]),
        }));
    }
    let resource_logs: Vec<Value> = by_host
        .into_iter()
        .map(|(host, records)| {
            json!({
                "resource": { "attributes": attrs_json(&[("host.name".into(), json!(host)), ("oc.signal".into(), json!("dd.connections"))]) },
                "scopeLogs": [{ "scope": { "name": "oc-tracegen" }, "logRecords": records }],
            })
        })
        .collect();
    json!({ "resourceLogs": resource_logs })
}

/// A stable share of the RTT as its variance: slower links vary more.
fn variance_share(rtt_us: f64) -> f64 {
    (rtt_us / 100_000.0).min(0.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_report_monitored_connections() {
        let p = payload(&mut Rng(3), 1, true);
        let records: usize = p["resourceLogs"].as_array().unwrap().iter().map(|r| r["scopeLogs"][0]["logRecords"].as_array().unwrap().len()).sum();
        let monitored = EDGES.iter().filter(|e| node(e.1).is_some()).count();
        assert_eq!(records, EDGES.len() + monitored + DNS_QUERIES.len());
    }
}
