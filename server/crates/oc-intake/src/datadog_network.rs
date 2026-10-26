//! Datadog Cloud Network Monitoring: the connections the Agent's system-probe sees
//! (`POST /api/v1/connections`, every 30 seconds per host). Each connection becomes an OTLP log
//! record with resource `oc.signal = dd.connections`, which the collector routes to
//! `otel.otel_network`; the network pages aggregate them into flows.
//!
//! Record attributes: direction (incoming / outgoing / local), type (tcp / udp), local.ip / port,
//! remote.ip / port (and remote.translated_ip / port behind NAT), whether the local port is ephemeral, pid, container ids,
//! the container's, host's and (same host) remote service's tags (local.tags, local.host_tags, remote.tags), bytes and packets sent
//! and received since the last report, retransmits, rtt and its variance (µs), and TCP connections opened / closed.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::datadog::inflate;
use crate::datadog_dns;
use crate::datadog_orch::message_body;
use crate::datadog_process::RES_COLLECTOR;
use crate::datadog_tags;
use crate::otlp;
use crate::{admit, send, Rejection, Shared};

const TYPE_COLLECTOR_CONNECTIONS: u8 = 22;

/// agent-payload proto/process/connections.proto (the fields used here).
pub mod pb {
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CollectorConnections {
        #[prost(string, tag = "2")]
        pub host_name: String,
        #[prost(message, repeated, tag = "3")]
        pub connections: Vec<Connection>,
        #[prost(map = "int32, string", tag = "10")]
        pub container_for_pid: HashMap<i32, String>,
        /// Host, container and service tag lists (see `datadog_tags`).
        #[prost(bytes = "vec", tag = "11")]
        pub encoded_tags: Vec<u8>,
        /// The reporting host's tags in `encoded_tags`.
        #[prost(int32, tag = "47")]
        pub host_tags_index: i32,
        /// IP → domains, version 1 encoding (older Agents).
        #[prost(bytes = "vec", tag = "14")]
        pub encoded_dns: Vec<u8>,
        /// Domains queried, for `dnsStatsByDomain*` keyed by index.
        #[prost(string, repeated, tag = "30")]
        pub domains: Vec<String>,
        #[prost(bytes = "vec", tag = "36")]
        pub encoded_domain_database: Vec<u8>,
        /// IP → domains, version 2 encoding (offsets into `encoded_domain_database`).
        #[prost(bytes = "vec", tag = "37")]
        pub encoded_dns_lookups: Vec<u8>,
    }
    /// DNS responses seen on a connection (to one domain and query type, or overall).
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DnsStats {
        #[prost(uint32, tag = "1")]
        pub timeouts: u32,
        /// Microseconds.
        #[prost(uint64, tag = "2")]
        pub success_latency_sum: u64,
        #[prost(uint64, tag = "3")]
        pub failure_latency_sum: u64,
        #[prost(map = "uint32, uint32", tag = "4")]
        pub count_by_rcode: HashMap<u32, u32>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DnsStatsByQueryType {
        #[prost(map = "int32, message", tag = "1")]
        pub by_query_type: HashMap<i32, DnsStats>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Connection {
        #[prost(int32, tag = "1")]
        pub pid: i32,
        #[prost(message, optional, tag = "5")]
        pub laddr: Option<Addr>,
        #[prost(message, optional, tag = "6")]
        pub raddr: Option<Addr>,
        #[prost(int32, tag = "10")]
        pub family: i32,
        #[prost(int32, tag = "11")]
        pub r#type: i32,
        #[prost(uint64, tag = "16")]
        pub last_bytes_sent: u64,
        #[prost(uint64, tag = "17")]
        pub last_bytes_received: u64,
        #[prost(uint32, tag = "18")]
        pub last_retransmits: u32,
        #[prost(int32, tag = "19")]
        pub direction: i32,
        #[prost(message, optional, tag = "21")]
        pub ip_translation: Option<IpTranslation>,
        #[prost(uint32, tag = "22")]
        pub rtt: u32,
        #[prost(uint32, tag = "23")]
        pub rtt_var: u32,
        #[prost(bool, tag = "24")]
        pub intra_host: bool,
        #[prost(uint32, tag = "27")]
        pub dns_timeouts: u32,
        #[prost(uint64, tag = "28")]
        pub dns_success_latency_sum: u64,
        #[prost(uint64, tag = "29")]
        pub dns_failure_latency_sum: u64,
        #[prost(map = "uint32, uint32", tag = "33")]
        pub dns_count_by_rcode: HashMap<u32, u32>,
        /// Deprecated: by index into `domains`, all query types together.
        #[prost(map = "int32, message", tag = "34")]
        pub dns_stats_by_domain: HashMap<i32, DnsStats>,
        /// By index into `domains`.
        #[prost(map = "int32, message", tag = "42")]
        pub dns_stats_by_domain_by_query_type: HashMap<i32, DnsStatsByQueryType>,
        /// By byte offset into `encoded_domain_database`.
        #[prost(map = "int32, message", tag = "43")]
        pub dns_stats_by_domain_offset_by_query_type: HashMap<i32, DnsStatsByQueryType>,
        #[prost(uint32, tag = "30")]
        pub last_tcp_established: u32,
        #[prost(uint32, tag = "31")]
        pub last_tcp_closed: u32,
        #[prost(uint64, tag = "38")]
        pub last_packets_sent: u64,
        #[prost(uint64, tag = "39")]
        pub last_packets_received: u64,
        #[prost(int32, tag = "41")]
        pub is_local_port_ephemeral: i32,
        /// The local container's tags in `encoded_tags` (-1: none).
        #[prost(int32, tag = "54")]
        pub local_container_tags_index: i32,
        /// For same-host connections, the remote service's tags in `encoded_tags` (-1: none).
        #[prost(int32, tag = "64")]
        pub remote_service_tags_index: i32,
    }
    /// NAT (e.g. a Kubernetes service address): the reply's source is the real server.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct IpTranslation {
        #[prost(string, tag = "1")]
        pub repl_src_ip: String,
        #[prost(string, tag = "2")]
        pub repl_dst_ip: String,
        #[prost(int32, tag = "3")]
        pub repl_src_port: i32,
        #[prost(int32, tag = "4")]
        pub repl_dst_port: i32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Addr {
        #[prost(string, tag = "2")]
        pub ip: String,
        #[prost(int32, tag = "3")]
        pub port: i32,
        #[prost(string, tag = "5")]
        pub container_id: String,
    }
}

/// The DNS stats of a connection: per (domain, query type) when the Agent breaks them down,
/// otherwise the connection's totals with no domain.
fn dns_stats(m: &pb::CollectorConnections, c: &pb::Connection) -> Vec<(String, Option<i32>, pb::DnsStats)> {
    let mut out = Vec::new();
    for (offset, by_type) in &c.dns_stats_by_domain_offset_by_query_type {
        let Some(domain) = usize::try_from(*offset).ok().and_then(|o| datadog_dns::domain_at(&m.encoded_domain_database, o)) else { continue };
        out.extend(by_type.by_query_type.iter().map(|(t, s)| (domain.clone(), Some(*t), s.clone())));
    }
    if out.is_empty() {
        for (idx, by_type) in &c.dns_stats_by_domain_by_query_type {
            let Some(domain) = usize::try_from(*idx).ok().and_then(|i| m.domains.get(i)) else { continue };
            out.extend(by_type.by_query_type.iter().map(|(t, s)| (domain.clone(), Some(*t), s.clone())));
        }
    }
    if out.is_empty() {
        for (idx, s) in &c.dns_stats_by_domain {
            let Some(domain) = usize::try_from(*idx).ok().and_then(|i| m.domains.get(i)) else { continue };
            out.push((domain.clone(), None, s.clone()));
        }
    }
    if out.is_empty() && (c.dns_timeouts > 0 || !c.dns_count_by_rcode.is_empty()) {
        out.push((
            String::new(),
            None,
            pb::DnsStats { timeouts: c.dns_timeouts, success_latency_sum: c.dns_success_latency_sum, failure_latency_sum: c.dns_failure_latency_sum, count_by_rcode: c.dns_count_by_rcode.clone() },
        ));
    }
    out.retain(|(_, _, s)| s.timeouts > 0 || s.count_by_rcode.values().any(|n| *n > 0));
    out
}

/// A DNS record's attributes: responses by outcome (NOERROR, SERVFAIL, NXDOMAIN, other errors),
/// timeouts and latency sums (µs).
fn dns_attributes(domain: &str, qtype: Option<i32>, s: &pb::DnsStats) -> Vec<otlp::KeyValue> {
    let count = |code: u32| u64::from(s.count_by_rcode.get(&code).copied().unwrap_or(0));
    let other: u64 = s.count_by_rcode.iter().filter(|(code, _)| ![0, 2, 3].contains(*code)).map(|(_, n)| u64::from(*n)).sum();
    let mut out = vec![
        otlp::kv("dns.noerror", count(0).to_string()),
        otlp::kv("dns.servfail", count(2).to_string()),
        otlp::kv("dns.nxdomain", count(3).to_string()),
        otlp::kv("dns.other_errors", other.to_string()),
        otlp::kv("dns.timeouts", s.timeouts.to_string()),
        otlp::kv("dns.success_latency_us", s.success_latency_sum.to_string()),
        otlp::kv("dns.failure_latency_us", s.failure_latency_sum.to_string()),
    ];
    if !domain.is_empty() {
        out.push(otlp::kv("dns.query", domain));
    }
    if let Some(t) = qtype {
        out.push(otlp::kv("dns.query_type", datadog_dns::query_type(t)));
    }
    out
}

fn direction(d: i32) -> &'static str {
    match d {
        1 => "incoming",
        2 => "outgoing",
        3 => "local",
        _ => "unspecified",
    }
}

/// OTLP log records for a CollectorConnections message (None for other message types).
pub fn to_otlp(data: &[u8], now: u64) -> Result<Option<(otlp::ExportLogsServiceRequest, usize)>, String> {
    use prost::Message;
    let (kind, raw) = message_body(data)?;
    if kind != TYPE_COLLECTOR_CONNECTIONS {
        return Ok(None);
    }
    let m = pb::CollectorConnections::decode(raw.as_slice()).map_err(|e| e.to_string())?;
    let containers: &HashMap<i32, String> = &m.container_for_pid;
    // What the host resolved: IP → domains.
    let resolved = if !m.encoded_dns_lookups.is_empty() {
        datadog_dns::decode_v2(&m.encoded_dns_lookups, &m.encoded_domain_database)
    } else if !m.encoded_dns.is_empty() {
        datadog_dns::decode_v1(&m.encoded_dns)
    } else {
        HashMap::new()
    };
    // The host's own tags (env, availability-zone, team, …), on every record of the payload.
    let host_tags = if m.encoded_tags.is_empty() { String::new() } else { datadog_tags::tags_at(&m.encoded_tags, m.host_tags_index).join(",") };
    let mut records: Vec<otlp::LogRecord> = Vec::new();
    for c in &m.connections {
        let l = c.laddr.clone().unwrap_or_default();
        let r = c.raddr.clone().unwrap_or_default();
        let container = if l.container_id.is_empty() { containers.get(&c.pid).cloned().unwrap_or_default() } else { l.container_id.clone() };
        // The endpoints, shared by the connection record and its DNS records.
        let mut endpoint = vec![
            otlp::kv("direction", direction(c.direction)),
            otlp::kv("type", if c.r#type == 1 { "udp" } else { "tcp" }),
            otlp::kv("family", if c.family == 1 { "v6" } else { "v4" }),
            otlp::kv("local.ip", l.ip.as_str()),
            otlp::kv("local.port", l.port.to_string()),
            otlp::kv("local.port_ephemeral", if c.is_local_port_ephemeral == 1 { "true" } else { "false" }),
            otlp::kv("remote.ip", r.ip.as_str()),
            otlp::kv("remote.port", r.port.to_string()),
            otlp::kv("pid", c.pid.to_string()),
        ];
        // The container's tags (service, kube_namespace, pod_name, image, …). Older Agents don't
        // set the index (it reads 0): only trust it for connections that have a container.
        if !container.is_empty() && !m.encoded_tags.is_empty() {
            let tags = datadog_tags::tags_at(&m.encoded_tags, c.local_container_tags_index);
            if !tags.is_empty() {
                endpoint.push(otlp::kv("local.tags", tags.join(",")));
            }
        }
        if !host_tags.is_empty() {
            endpoint.push(otlp::kv("local.host_tags", host_tags.as_str()));
        }
        // Same-host connections name the remote service; index 0 only counts with a remote container
        // (older Agents leave it unset).
        let remote_index = c.remote_service_tags_index > 0 || (c.remote_service_tags_index == 0 && !r.container_id.is_empty());
        if c.intra_host && remote_index && !m.encoded_tags.is_empty() {
            let tags = datadog_tags::tags_at(&m.encoded_tags, c.remote_service_tags_index);
            if !tags.is_empty() {
                endpoint.push(otlp::kv("remote.tags", tags.join(",")));
            }
        }
        if !container.is_empty() {
            endpoint.push(otlp::kv("container.id", container));
        }
        if let Some(domain) = resolved.get(&r.ip).and_then(|d| d.first()) {
            endpoint.push(otlp::kv("remote.domain", domain.as_str()));
        }
        for (domain, qtype, stats) in dns_stats(&m, c) {
            let mut attributes = endpoint.clone();
            attributes.push(otlp::kv("kind", "dns"));
            attributes.extend(dns_attributes(&domain, qtype, &stats));
            records.push(otlp::LogRecord { time_unix_nano: now, observed_time_unix_nano: now, attributes, ..Default::default() });
        }
        // Nothing moved since the last report: not a flow worth storing.
        if c.last_bytes_sent + c.last_bytes_received + u64::from(c.last_tcp_established) + u64::from(c.last_tcp_closed) == 0 {
            continue;
        }
        records.push({
            let mut attributes = endpoint;
            attributes.extend([
                otlp::kv("type", if c.r#type == 1 { "udp" } else { "tcp" }),
                otlp::kv("family", if c.family == 1 { "v6" } else { "v4" }),
                otlp::kv("local.ip", l.ip.as_str()),
                otlp::kv("local.port", l.port.to_string()),
                otlp::kv("local.port_ephemeral", if c.is_local_port_ephemeral == 1 { "true" } else { "false" }),
                otlp::kv("remote.ip", r.ip.as_str()),
                otlp::kv("remote.port", r.port.to_string()),
                otlp::kv("pid", c.pid.to_string()),
                otlp::kv("bytes_sent", c.last_bytes_sent.to_string()),
                otlp::kv("bytes_received", c.last_bytes_received.to_string()),
                otlp::kv("packets_sent", c.last_packets_sent.to_string()),
                otlp::kv("packets_received", c.last_packets_received.to_string()),
                otlp::kv("retransmits", c.last_retransmits.to_string()),
                otlp::kv("rtt_us", c.rtt.to_string()),
                otlp::kv("rtt_var_us", c.rtt_var.to_string()),
                otlp::kv("tcp_established", c.last_tcp_established.to_string()),
                otlp::kv("tcp_closed", c.last_tcp_closed.to_string()),
                otlp::kv("intra_host", c.intra_host.to_string()),
            ]);
            if !r.container_id.is_empty() {
                attributes.push(otlp::kv("remote.container.id", r.container_id.as_str()));
            }
            // Behind NAT (a Kubernetes service): the address that actually answered.
            if let Some(t) = c.ip_translation.as_ref().filter(|t| !t.repl_src_ip.is_empty() && t.repl_src_ip != r.ip) {
                attributes.push(otlp::kv("remote.translated_ip", t.repl_src_ip.as_str()));
                attributes.push(otlp::kv("remote.translated_port", t.repl_src_port.to_string()));
            }
            otlp::LogRecord { time_unix_nano: now, observed_time_unix_nano: now, attributes, ..Default::default() }
        });
    }
    let n = records.len();
    let attributes = vec![otlp::kv("host.name", m.host_name.as_str()), otlp::kv("oc.signal", "dd.connections")];
    let req = otlp::ExportLogsServiceRequest {
        resource_logs: vec![otlp::ResourceLogs { resource: Some(otlp::Resource { attributes }), scope_logs: vec![otlp::ScopeLogs { scope: otlp::scope("datadog.network"), log_records: records }] }],
    };
    Ok(Some((req, n)))
}

/// `POST /api/v1/connections`.
pub async fn connections(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let converted = inflate(&headers, &body).and_then(|d| to_otlp(&d, now));
    drop(body);
    match converted {
        Ok(Some((req, n))) if n > 0 => {
            let target = format!("{}/v1/logs", st.cfg.collector_url);
            match send(&st, &org, &target, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&req)).await {
                Ok(up) if up.status.is_success() => {}
                Ok(up) => {
                    tracing::warn!(status = %up.status, "collector refused converted Datadog connections");
                    return Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "connections were not accepted" }.into_response();
                }
                Err(r) => return r.into_response(),
            }
        }
        Ok(_) => {}
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    }
    ([(header::CONTENT_TYPE, "application/x-protobuf")], Bytes::from_static(RES_COLLECTOR)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn connections_become_records() {
        let mut containers = HashMap::new();
        containers.insert(7, "c-checkout".to_string());
        let m = pb::CollectorConnections {
            host_name: "web-1".into(),
            connections: vec![
                pb::Connection {
                    pid: 7,
                    laddr: Some(pb::Addr { ip: "10.0.0.5".into(), port: 51234, ..Default::default() }),
                    raddr: Some(pb::Addr { ip: "10.0.0.9".into(), port: 5432, ..Default::default() }),
                    direction: 2,
                    last_bytes_sent: 1200,
                    last_bytes_received: 64_000,
                    rtt: 350,
                    is_local_port_ephemeral: 1,
                    ..Default::default()
                },
                // Idle since the last report: dropped.
                pb::Connection { pid: 8, ..Default::default() },
            ],
            container_for_pid: containers,
            ..Default::default()
        };
        let mut msg = vec![3, 4, TYPE_COLLECTOR_CONNECTIONS, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        msg.extend_from_slice(&zstd::encode_all(m.encode_to_vec().as_slice(), 1).unwrap());
        let (req, n) = to_otlp(&msg, 5).unwrap().unwrap();
        assert_eq!(n, 1);
        // No encoded tags: no tag attributes.
        assert!(!req.resource_logs[0].scope_logs[0].log_records[0].attributes.iter().any(|kv| kv.key == "local.tags"));
        let r = &req.resource_logs[0].scope_logs[0].log_records[0];
        for (k, v) in [("direction", "outgoing"), ("remote.port", "5432"), ("local.port_ephemeral", "true"), ("container.id", "c-checkout"), ("bytes_received", "64000"), ("rtt_us", "350")] {
            assert!(r.attributes.contains(&otlp::kv(k, v)), "{k}");
        }
        assert!(req.resource_logs[0].resource.as_ref().unwrap().attributes.contains(&otlp::kv("oc.signal", "dd.connections")));
    }

    #[test]
    fn dns_lookups_and_stats() {
        let (lookups, db) = datadog_dns::tests::encode_v2(&[("203.0.113.10", &["payments.example.net"])]);
        let mut rcodes = HashMap::new();
        rcodes.insert(0, 40);
        rcodes.insert(3, 2);
        let mut by_type = HashMap::new();
        by_type.insert(1, pb::DnsStats { timeouts: 1, success_latency_sum: 4000, failure_latency_sum: 300, count_by_rcode: rcodes });
        let mut by_domain = HashMap::new();
        // The domain's offset in the database: after the count and middle varints.
        by_domain.insert(2, pb::DnsStatsByQueryType { by_query_type: by_type });
        let m = pb::CollectorConnections {
            host_name: "web-1".into(),
            connections: vec![
                pb::Connection {
                    laddr: Some(pb::Addr { ip: "10.0.0.5".into(), port: 40000, ..Default::default() }),
                    raddr: Some(pb::Addr { ip: "10.0.0.2".into(), port: 53, ..Default::default() }),
                    r#type: 1,
                    direction: 2,
                    last_bytes_sent: 3000,
                    last_bytes_received: 9000,
                    dns_stats_by_domain_offset_by_query_type: by_domain,
                    ..Default::default()
                },
                pb::Connection {
                    laddr: Some(pb::Addr { ip: "10.0.0.5".into(), port: 41000, ..Default::default() }),
                    raddr: Some(pb::Addr { ip: "203.0.113.10".into(), port: 443, ..Default::default() }),
                    direction: 2,
                    last_bytes_sent: 100,
                    ..Default::default()
                },
            ],
            encoded_dns_lookups: lookups,
            encoded_domain_database: db,
            ..Default::default()
        };
        let mut msg = vec![3, 4, TYPE_COLLECTOR_CONNECTIONS, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        msg.extend_from_slice(&zstd::encode_all(m.encode_to_vec().as_slice(), 1).unwrap());
        let (req, n) = to_otlp(&msg, 5).unwrap().unwrap();
        assert_eq!(n, 3);
        let records = &req.resource_logs[0].scope_logs[0].log_records;
        let dns = records.iter().find(|r| r.attributes.contains(&otlp::kv("kind", "dns"))).unwrap();
        for (k, v) in [("dns.query", "payments.example.net"), ("dns.query_type", "A"), ("dns.noerror", "40"), ("dns.nxdomain", "2"), ("dns.timeouts", "1"), ("remote.port", "53")] {
            assert!(dns.attributes.contains(&otlp::kv(k, v)), "{k}");
        }
        let external = records.iter().find(|r| r.attributes.contains(&otlp::kv("remote.ip", "203.0.113.10"))).unwrap();
        assert!(external.attributes.contains(&otlp::kv("remote.domain", "payments.example.net")));
    }

    #[test]
    fn container_and_host_tags() {
        let (tags, idx) = datadog_tags::tests::encode_v2(&[&["env:prod", "availability-zone:us-east-1a"], &["service:checkout", "kube_namespace:shop", "pod_name:checkout-7d9f"]]);
        let mut containers = HashMap::new();
        containers.insert(7, "c-checkout".to_string());
        let m = pb::CollectorConnections {
            host_name: "web-1".into(),
            connections: vec![
                pb::Connection {
                    pid: 7,
                    laddr: Some(pb::Addr { ip: "10.0.0.5".into(), port: 40000, ..Default::default() }),
                    raddr: Some(pb::Addr { ip: "10.0.0.9".into(), port: 8080, ..Default::default() }),
                    direction: 2,
                    last_bytes_sent: 10,
                    local_container_tags_index: idx[1],
                    remote_service_tags_index: -1,
                    ..Default::default()
                },
                // A host process: no container, so no container tags even with index 0.
                pb::Connection {
                    pid: 9,
                    laddr: Some(pb::Addr { ip: "10.0.0.5".into(), port: 40001, ..Default::default() }),
                    raddr: Some(pb::Addr { ip: "10.0.0.9".into(), port: 22, ..Default::default() }),
                    direction: 2,
                    last_bytes_sent: 10,
                    ..Default::default()
                },
            ],
            container_for_pid: containers,
            encoded_tags: tags,
            host_tags_index: idx[0],
            ..Default::default()
        };
        let mut msg = vec![3, 4, TYPE_COLLECTOR_CONNECTIONS, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        msg.extend_from_slice(&zstd::encode_all(m.encode_to_vec().as_slice(), 1).unwrap());
        let (req, _) = to_otlp(&msg, 5).unwrap().unwrap();
        let records = &req.resource_logs[0].scope_logs[0].log_records;
        assert!(records[0].attributes.contains(&otlp::kv("local.tags", "service:checkout,kube_namespace:shop,pod_name:checkout-7d9f")));
        assert!(records[0].attributes.contains(&otlp::kv("local.host_tags", "env:prod,availability-zone:us-east-1a")));
        assert!(!records[1].attributes.iter().any(|kv| kv.key == "local.tags"));
        assert!(records[1].attributes.contains(&otlp::kv("local.host_tags", "env:prod,availability-zone:us-east-1a")));
    }
}
