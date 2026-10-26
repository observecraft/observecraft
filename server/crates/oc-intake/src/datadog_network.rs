//! Datadog Cloud Network Monitoring: the connections the Agent's system-probe sees
//! (`POST /api/v1/connections`, every 30 seconds per host). Each connection becomes an OTLP log
//! record with resource `oc.signal = dd.connections`, which the collector routes to
//! `otel.otel_network`; the network pages aggregate them into flows.
//!
//! Record attributes: direction (incoming / outgoing / local), type (tcp / udp), local.ip / port,
//! remote.ip / port (and remote.translated_ip / port behind NAT), whether the local port is ephemeral, pid, container ids, bytes and packets sent
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
use crate::datadog_orch::message_body;
use crate::datadog_process::RES_COLLECTOR;
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
    let records: Vec<otlp::LogRecord> = m
        .connections
        .iter()
        // Nothing moved since the last report: not a flow worth storing.
        .filter(|c| c.last_bytes_sent + c.last_bytes_received + u64::from(c.last_tcp_established) + u64::from(c.last_tcp_closed) > 0)
        .map(|c| {
            let l = c.laddr.clone().unwrap_or_default();
            let r = c.raddr.clone().unwrap_or_default();
            let container = if l.container_id.is_empty() { containers.get(&c.pid).cloned().unwrap_or_default() } else { l.container_id.clone() };
            let mut attributes = vec![
                otlp::kv("direction", direction(c.direction)),
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
            ];
            if !container.is_empty() {
                attributes.push(otlp::kv("container.id", container));
            }
            if !r.container_id.is_empty() {
                attributes.push(otlp::kv("remote.container.id", r.container_id.as_str()));
            }
            // Behind NAT (a Kubernetes service): the address that actually answered.
            if let Some(t) = c.ip_translation.as_ref().filter(|t| !t.repl_src_ip.is_empty() && t.repl_src_ip != r.ip) {
                attributes.push(otlp::kv("remote.translated_ip", t.repl_src_ip.as_str()));
                attributes.push(otlp::kv("remote.translated_port", t.repl_src_port.to_string()));
            }
            otlp::LogRecord { time_unix_nano: now, observed_time_unix_nano: now, attributes, ..Default::default() }
        })
        .collect();
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
        };
        let mut msg = vec![3, 4, TYPE_COLLECTOR_CONNECTIONS, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        msg.extend_from_slice(&zstd::encode_all(m.encode_to_vec().as_slice(), 1).unwrap());
        let (req, n) = to_otlp(&msg, 5).unwrap().unwrap();
        assert_eq!(n, 1);
        let r = &req.resource_logs[0].scope_logs[0].log_records[0];
        for (k, v) in [("direction", "outgoing"), ("remote.port", "5432"), ("local.port_ephemeral", "true"), ("container.id", "c-checkout"), ("bytes_received", "64000"), ("rtt_us", "350")] {
            assert!(r.attributes.contains(&otlp::kv(k, v)), "{k}");
        }
        assert!(req.resource_logs[0].resource.as_ref().unwrap().attributes.contains(&otlp::kv("oc.signal", "dd.connections")));
    }
}
