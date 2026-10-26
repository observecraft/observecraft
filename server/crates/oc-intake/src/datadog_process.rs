//! Datadog Agent live processes and containers (`/api/v1/collector`, `/api/v1/container`, sent with
//! `process_config.process_dd_url`). Each snapshot becomes OTLP metrics under the same names and
//! resource attributes the OpenTelemetry hostmetrics `process` scraper uses, so the host page's
//! Processes tab (and anything querying `process.*` / `container.*`) reads Datadog data too:
//!   process.cpu.utilization (fraction of one core), process.memory.usage (RSS),
//!   process.memory.virtual, process.threads, process.open_file_descriptors;
//!   container.cpu.usage (cores), container.memory.usage, container.cpu.limit, container.memory.limit.
//!
//! Payloads are agent-payload "process" messages (header + zstd protobuf). The Agent expects an
//! agent-payload `ResCollector` back.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::datadog::{inflate, TAG_ATTRS};
use crate::datadog_orch::message_body;
use crate::otlp::{self, KeyValue};
use crate::{admit, send, Rejection, Shared};

const TYPE_COLLECTOR_PROC: u8 = 12;
const TYPE_COLLECTOR_CONTAINER: u8 = 39;

/// The answer the Agent expects: a `ResCollector` (header v1, protobuf, type 23) with
/// `status { activeClients: 0, interval: 10 }` (no real-time mode).
pub const RES_COLLECTOR: &[u8] = &[1, 0, 23, 0, 0x1a, 0x02, 0x10, 0x0a];

/// agent-payload proto/process/agent.proto (the fields used here).
pub mod pb {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CollectorProc {
        #[prost(string, tag = "2")]
        pub host_name: String,
        #[prost(message, repeated, tag = "3")]
        pub processes: Vec<Process>,
        #[prost(message, repeated, tag = "10")]
        pub containers: Vec<Container>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CollectorContainer {
        #[prost(string, tag = "1")]
        pub host_name: String,
        #[prost(message, repeated, tag = "3")]
        pub containers: Vec<Container>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Process {
        #[prost(int32, tag = "2")]
        pub pid: i32,
        #[prost(message, optional, tag = "4")]
        pub command: Option<Command>,
        #[prost(message, optional, tag = "5")]
        pub user: Option<ProcessUser>,
        #[prost(message, optional, tag = "7")]
        pub memory: Option<MemoryStat>,
        #[prost(message, optional, tag = "8")]
        pub cpu: Option<CpuStat>,
        #[prost(int64, tag = "9")]
        pub create_time: i64,
        #[prost(int32, tag = "11")]
        pub open_fd_count: i32,
        #[prost(string, tag = "14")]
        pub container_id: String,
        #[prost(string, repeated, tag = "23")]
        pub tags: Vec<String>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Command {
        #[prost(string, repeated, tag = "1")]
        pub args: Vec<String>,
        #[prost(string, tag = "3")]
        pub cwd: String,
        #[prost(int32, tag = "6")]
        pub ppid: i32,
        #[prost(string, tag = "8")]
        pub exe: String,
        #[prost(string, tag = "9")]
        pub comm: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ProcessUser {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(int32, tag = "2")]
        pub uid: i32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MemoryStat {
        #[prost(uint64, tag = "1")]
        pub rss: u64,
        #[prost(uint64, tag = "2")]
        pub vms: u64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CpuStat {
        #[prost(float, tag = "2")]
        pub total_pct: f32,
        #[prost(int32, tag = "5")]
        pub num_threads: i32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Container {
        #[prost(string, tag = "1")]
        pub r#type: String,
        #[prost(string, tag = "2")]
        pub id: String,
        #[prost(string, tag = "3")]
        pub name: String,
        #[prost(string, tag = "4")]
        pub image: String,
        #[prost(float, tag = "5")]
        pub cpu_limit: f32,
        #[prost(uint64, tag = "6")]
        pub memory_limit: u64,
        #[prost(float, tag = "20")]
        pub total_pct: f32,
        #[prost(uint64, tag = "21")]
        pub mem_rss: u64,
        #[prost(string, repeated, tag = "26")]
        pub tags: Vec<String>,
        #[prost(uint64, tag = "30")]
        pub mem_usage: u64,
    }
}

fn gauge(name: &str, unit: &str, t: u64, v: f64) -> otlp::Metric {
    otlp::Metric {
        name: name.into(),
        unit: unit.into(),
        data: Some(otlp::metric::Data::Gauge(otlp::Gauge { data_points: vec![otlp::NumberDataPoint { start_time_unix_nano: 0, time_unix_nano: t, as_double: v, attributes: vec![] }] })),
    }
}

/// A non-monotonic cumulative sum (how hostmetrics reports memory).
fn updown(name: &str, unit: &str, t: u64, v: f64) -> otlp::Metric {
    let p = otlp::NumberDataPoint { start_time_unix_nano: 0, time_unix_nano: t, as_double: v, attributes: vec![] };
    otlp::Metric { name: name.into(), unit: unit.into(), data: Some(otlp::metric::Data::Sum(otlp::Sum { data_points: vec![p], aggregation_temporality: 2, is_monotonic: false })) }
}

/// `key:value` tags: Kubernetes / container tags become their OTel resource attributes.
fn tag_attrs(tags: &[String], out: &mut BTreeMap<String, String>) {
    for tag in tags {
        if let Some((k, v)) = tag.split_once(':') {
            if let Some((_, otel)) = TAG_ATTRS.iter().find(|(dd, _)| *dd == k) {
                out.entry((*otel).to_string()).or_insert_with(|| v.to_string());
            }
        }
    }
}

fn resource(attrs: BTreeMap<String, String>) -> Option<otlp::Resource> {
    let mut attributes: Vec<KeyValue> = attrs.into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| otlp::kv(k, v)).collect();
    attributes.push(otlp::kv("observecraft.collector", "datadog-agent"));
    Some(otlp::Resource { attributes })
}

fn process_metrics(host: &str, p: &pb::Process, t: u64) -> otlp::ResourceMetrics {
    let cmd = p.command.clone().unwrap_or_default();
    let name = if cmd.comm.is_empty() { cmd.exe.rsplit('/').next().unwrap_or_default().to_string() } else { cmd.comm.clone() };
    let mut a = BTreeMap::new();
    a.insert("host.name".to_string(), host.to_string());
    a.insert("process.pid".to_string(), p.pid.to_string());
    a.insert("process.parent_pid".to_string(), cmd.ppid.to_string());
    a.insert("process.executable.name".to_string(), name);
    a.insert("process.executable.path".to_string(), cmd.exe.clone());
    a.insert("process.command".to_string(), cmd.args.first().cloned().unwrap_or_default());
    a.insert("process.command_line".to_string(), cmd.args.join(" "));
    // The user name, or the uid when the host has no name for it (container users).
    let owner = p.user.as_ref().map(|u| if u.name.is_empty() { u.uid.to_string() } else { u.name.clone() }).unwrap_or_default();
    a.insert("process.owner".to_string(), owner);
    a.insert("container.id".to_string(), p.container_id.clone());
    if p.create_time > 0 {
        // Milliseconds since the epoch.
        a.insert("process.create_time".to_string(), p.create_time.to_string());
    }
    tag_attrs(&p.tags, &mut a);
    let mut metrics = Vec::new();
    if let Some(c) = &p.cpu {
        metrics.push(gauge("process.cpu.utilization", "1", t, f64::from(c.total_pct) / 100.0));
        metrics.push(gauge("process.threads", "{threads}", t, f64::from(c.num_threads)));
    }
    if let Some(m) = &p.memory {
        metrics.push(updown("process.memory.usage", "By", t, m.rss as f64));
        metrics.push(updown("process.memory.virtual", "By", t, m.vms as f64));
    }
    if p.open_fd_count > 0 {
        metrics.push(gauge("process.open_file_descriptors", "{count}", t, f64::from(p.open_fd_count)));
    }
    otlp::ResourceMetrics { resource: resource(a), scope_metrics: vec![otlp::ScopeMetrics { scope: otlp::scope("datadog.process"), metrics }] }
}

fn container_metrics(host: &str, c: &pb::Container, t: u64) -> otlp::ResourceMetrics {
    let mut a = BTreeMap::new();
    a.insert("host.name".to_string(), host.to_string());
    a.insert("container.id".to_string(), c.id.clone());
    a.insert("container.name".to_string(), c.name.clone());
    a.insert("container.image.name".to_string(), c.image.clone());
    a.insert("container.runtime".to_string(), c.r#type.clone());
    tag_attrs(&c.tags, &mut a);
    let mut metrics = vec![gauge("container.cpu.usage", "{cpu}", t, f64::from(c.total_pct) / 100.0)];
    let mem = if c.mem_usage > 0 { c.mem_usage } else { c.mem_rss };
    metrics.push(gauge("container.memory.usage", "By", t, mem as f64));
    if c.cpu_limit > 0.0 {
        // The Agent reports the limit in percent of one core.
        metrics.push(gauge("container.cpu.limit", "{cpu}", t, f64::from(c.cpu_limit) / 100.0));
    }
    if c.memory_limit > 0 {
        metrics.push(gauge("container.memory.limit", "By", t, c.memory_limit as f64));
    }
    otlp::ResourceMetrics { resource: resource(a), scope_metrics: vec![otlp::ScopeMetrics { scope: otlp::scope("datadog.container"), metrics }] }
}

/// OTLP metrics for a CollectorProc / CollectorContainer message; `None` for other message types.
pub fn to_otlp(data: &[u8], now: u64) -> Result<Option<(otlp::ExportMetricsServiceRequest, usize)>, String> {
    use prost::Message;
    let (kind, raw) = message_body(data)?;
    let (host, processes, containers) = match kind {
        TYPE_COLLECTOR_PROC => {
            let m = pb::CollectorProc::decode(raw.as_slice()).map_err(|e| e.to_string())?;
            (m.host_name, m.processes, m.containers)
        }
        TYPE_COLLECTOR_CONTAINER => {
            let m = pb::CollectorContainer::decode(raw.as_slice()).map_err(|e| e.to_string())?;
            (m.host_name, vec![], m.containers)
        }
        _ => return Ok(None),
    };
    let n = processes.len() + containers.len();
    let mut resource_metrics: Vec<otlp::ResourceMetrics> = processes.iter().map(|p| process_metrics(&host, p, now)).collect();
    resource_metrics.extend(containers.iter().filter(|c| !c.id.is_empty()).map(|c| container_metrics(&host, c, now)));
    Ok(Some((otlp::ExportMetricsServiceRequest { resource_metrics }, n)))
}

/// `POST /api/v1/collector`, `/api/v1/container`.
pub async fn collector(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let converted = inflate(&headers, &body).and_then(|d| to_otlp(&d, now));
    drop(body);
    match converted {
        Ok(Some((req, n))) if n > 0 => {
            let target = format!("{}/v1/metrics", st.cfg.collector_url);
            match send(&st, &org, &target, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&req)).await {
                Ok(up) if up.status.is_success() => {}
                Ok(up) => {
                    tracing::warn!(status = %up.status, "collector refused converted Datadog processes");
                    return Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "processes were not accepted" }.into_response();
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

    fn message(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![3, 4, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&zstd::encode_all(body, 1).unwrap());
        out
    }

    fn attr<'a>(r: &'a otlp::ResourceMetrics, key: &str) -> &'a str {
        r.resource.as_ref().unwrap().attributes.iter().find(|a| a.key == key).map(otlp::text).unwrap_or("")
    }

    #[test]
    fn processes_become_process_metrics() {
        let m = pb::CollectorProc {
            host_name: "web-1".into(),
            processes: vec![pb::Process {
                pid: 42,
                command: Some(pb::Command {
                    args: vec!["/usr/bin/order-service".into(), "--port".into(), "8080".into()],
                    ppid: 1,
                    exe: "/usr/bin/order-service".into(),
                    comm: "order-service".into(),
                    ..Default::default()
                }),
                user: Some(pb::ProcessUser { name: "app".into(), uid: 1000 }),
                memory: Some(pb::MemoryStat { rss: 50_000_000, vms: 900_000_000 }),
                cpu: Some(pb::CpuStat { total_pct: 150.0, num_threads: 12 }),
                tags: vec!["kube_namespace:shop".into()],
                ..Default::default()
            }],
            containers: vec![pb::Container { id: "c1".into(), total_pct: 50.0, mem_usage: 1024, cpu_limit: 200.0, tags: vec!["pod_name:order-7d9".into()], ..Default::default() }],
        };
        let (req, n) = to_otlp(&message(TYPE_COLLECTOR_PROC, &m.encode_to_vec()), 1).unwrap().unwrap();
        assert_eq!(n, 2);
        let p = &req.resource_metrics[0];
        assert_eq!(attr(p, "process.pid"), "42");
        assert_eq!(attr(p, "process.executable.name"), "order-service");
        assert_eq!(attr(p, "process.command_line"), "/usr/bin/order-service --port 8080");
        assert_eq!(attr(p, "process.owner"), "app");
        assert_eq!(attr(p, "k8s.namespace.name"), "shop");
        assert_eq!(attr(p, "observecraft.collector"), "datadog-agent");
        let cpu = &p.scope_metrics[0].metrics[0];
        assert_eq!(cpu.name, "process.cpu.utilization");
        let Some(otlp::metric::Data::Gauge(g)) = &cpu.data else { panic!() };
        assert_eq!(g.data_points[0].as_double, 1.5);
        assert!(p.scope_metrics[0].metrics.iter().any(|m| m.name == "process.memory.usage"));
        let c = &req.resource_metrics[1];
        assert_eq!(attr(c, "container.id"), "c1");
        assert_eq!(attr(c, "k8s.pod.name"), "order-7d9");
        let Some(otlp::metric::Data::Gauge(limit)) = &c.scope_metrics[0].metrics[2].data else { panic!() };
        assert_eq!(limit.data_points[0].as_double, 2.0);
        // Other message types are not processes.
        assert!(to_otlp(&message(41, b""), 1).unwrap().is_none());
    }
}
