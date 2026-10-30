//! Datadog Orchestrator Explorer intake: what the Datadog Cluster Agent (and node Agents) send with
//! `orchestrator_explorer.enabled` (`orchestrator_explorer.orchestrator_dd_url`).
//!
//! - `POST /api/v2/orchmanif`  full Kubernetes manifests (CollectorManifest, CRDs, custom resources) →
//!   the same object snapshots the observecraft-agent's cluster agent sends, so the Kubernetes
//!   Explorer (tables, YAML) works from Datadog's agents too
//! - `POST /api/v2/orch`       typed resource summaries: acknowledged (the manifests carry everything)
//!
//! Payloads are agent-payload "process" messages: a small header (version, encoding, type, …)
//! followed by a protobuf body, zstd-compressed for most encodings.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::datadog::inflate;
use crate::otlp;
use crate::{admit, send, Rejection, Shared};

/// agent-payload proto/process/agent.proto (the parts manifests need).
pub mod pb {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CollectorManifest {
        #[prost(string, tag = "1")]
        pub cluster_name: String,
        #[prost(string, tag = "2")]
        pub cluster_id: String,
        #[prost(message, repeated, tag = "5")]
        pub manifests: Vec<Manifest>,
        #[prost(string, repeated, tag = "6")]
        pub tags: Vec<String>,
        #[prost(string, tag = "7")]
        pub host_name: String,
    }
    /// CollectorManifestCRD / CollectorManifestCR: a CollectorManifest wrapped with tags.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Wrapped {
        #[prost(message, optional, tag = "1")]
        pub manifest: Option<CollectorManifest>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Manifest {
        #[prost(int32, tag = "1")]
        pub r#type: i32,
        #[prost(string, tag = "2")]
        pub resource_version: String,
        #[prost(string, tag = "3")]
        pub uid: String,
        #[prost(bytes = "vec", tag = "4")]
        pub content: Vec<u8>,
        #[prost(string, tag = "5")]
        pub content_type: String,
        #[prost(string, tag = "6")]
        pub version: String,
        #[prost(bool, tag = "8")]
        pub is_terminated: bool,
        #[prost(string, tag = "9")]
        pub api_version: String,
        #[prost(string, tag = "10")]
        pub node_name: String,
        #[prost(string, tag = "11")]
        pub kind: String,
    }
}

const TYPE_MANIFEST: u8 = 80;
const TYPE_MANIFEST_CRD: u8 = 81;
const TYPE_MANIFEST_CR: u8 = 82;

/// Message header (agent-payload process/message.go): returns (encoding, type, body offset).
pub fn read_header(data: &[u8]) -> Result<(u8, u8, usize), String> {
    if data.len() <= 4 {
        return Err(format!("invalid message length: {}", data.len()));
    }
    let len = match data[0] {
        1 => 4,
        2 => 8,
        3 => 16,
        v => return Err(format!("invalid message version: {v}")),
    };
    if data.len() < len {
        return Err("truncated message header".into());
    }
    Ok((data[1], data[2], len))
}

/// The protobuf body: encodings 2, 4 and 5 are zstd-compressed, 0 is plain.
fn body(encoding: u8, raw: &[u8]) -> Result<Vec<u8>, String> {
    match encoding {
        0 => Ok(raw.to_vec()),
        2 | 4 | 5 => zstd::stream::decode_all(raw).map_err(|e| format!("zstd: {e}")),
        e => Err(format!("unsupported message encoding {e}")),
    }
}

/// Kubernetes kinds of the orchestrator message types, for manifests that don't name theirs.
fn kind_of_type(t: i32) -> &'static str {
    match t {
        41 => "Pod",
        42 => "ReplicaSet",
        43 => "Deployment",
        44 => "Service",
        45 => "Node",
        47 => "Job",
        48 => "CronJob",
        49 => "DaemonSet",
        50 => "StatefulSet",
        51 => "PersistentVolume",
        52 => "PersistentVolumeClaim",
        54 => "Role",
        55 => "RoleBinding",
        56 => "ClusterRole",
        57 => "ClusterRoleBinding",
        58 => "ServiceAccount",
        59 => "Ingress",
        61 => "Namespace",
        81 => "CustomResourceDefinition",
        83 => "VerticalPodAutoscaler",
        84 => "HorizontalPodAutoscaler",
        85 => "NetworkPolicy",
        86 => "LimitRange",
        87 => "StorageClass",
        88 => "PodDisruptionBudget",
        _ => "",
    }
}

/// A message's type and its protobuf body (decompressed).
pub fn message_body(data: &[u8]) -> Result<(u8, Vec<u8>), String> {
    let (encoding, kind, offset) = read_header(data)?;
    Ok((kind, body(encoding, &data[offset..])?))
}

/// The CollectorManifest in a message (plain or wrapped).
pub fn decode(data: &[u8]) -> Result<pb::CollectorManifest, String> {
    let (kind, raw) = message_body(data)?;
    use prost::Message;
    match kind {
        TYPE_MANIFEST => pb::CollectorManifest::decode(raw.as_slice()).map_err(|e| e.to_string()),
        TYPE_MANIFEST_CRD | TYPE_MANIFEST_CR => pb::Wrapped::decode(raw.as_slice()).map_err(|e| e.to_string())?.manifest.ok_or_else(|| "empty manifest".to_string()),
        t => Err(format!("not a manifest message (type {t})")),
    }
}

/// Object snapshots as OTLP logs, marked `oc.signal = k8s.objects` like the cluster agent's.
pub fn to_otlp(m: &pb::CollectorManifest) -> (otlp::ExportLogsServiceRequest, usize) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let log_records: Vec<otlp::LogRecord> = m
        .manifests
        .iter()
        .filter(|x| !x.is_terminated)
        .filter_map(|x| {
            let mut obj: Value = serde_json::from_slice(&x.content).ok()?;
            let o = obj.as_object_mut()?;
            // Typed objects marshal without their TypeMeta; the manifest carries it.
            if o.get("kind").and_then(Value::as_str).unwrap_or("").is_empty() {
                let kind = if x.kind.is_empty() { kind_of_type(x.r#type) } else { x.kind.as_str() };
                o.insert("kind".into(), json!(kind));
            }
            if o.get("apiVersion").and_then(Value::as_str).unwrap_or("").is_empty() && !x.api_version.is_empty() {
                o.insert("apiVersion".into(), json!(x.api_version));
            }
            let body = otlp::AnyValue { value: Some(otlp::any_value::Value::StringValue(obj.to_string())) };
            Some(otlp::LogRecord { time_unix_nano: now, observed_time_unix_nano: now, body: Some(body), ..Default::default() })
        })
        .collect();
    let n = log_records.len();
    let attributes = vec![otlp::kv("k8s.cluster.name", m.cluster_name.as_str()), otlp::kv("oc.signal", "k8s.objects"), otlp::kv("observecraft.collector", "datadog-cluster-agent")];
    let req = otlp::ExportLogsServiceRequest {
        resource_logs: vec![otlp::ResourceLogs { resource: Some(otlp::Resource { attributes }), scope_logs: vec![otlp::ScopeLogs { scope: otlp::scope("datadog.orchestrator"), log_records }] }],
    };
    (req, n)
}

pub async fn manifests(State(st): State<Shared>, headers: HeaderMap, raw: Bytes) -> Response {
    let org = match admit(&st, &headers, raw.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let manifest = match inflate(&headers, &raw).and_then(|d| decode(&d)) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [e] }))).into_response(),
    };
    drop(raw);
    let (req, n) = to_otlp(&manifest);
    tracing::debug!(objects = n, cluster = %manifest.cluster_name, "datadog manifests");
    if n == 0 {
        return StatusCode::ACCEPTED.into_response();
    }
    let target = crate::kafka::Sink::Logs;
    match send(&st, &org, target, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&req)).await {
        Ok(up) if up.status.is_success() => StatusCode::ACCEPTED.into_response(),
        Ok(up) => {
            tracing::warn!(status = %up.status, "collector refused converted manifests");
            Rejection { status: if up.status.is_client_error() { StatusCode::BAD_REQUEST } else { StatusCode::SERVICE_UNAVAILABLE }, message: "manifests were not accepted" }.into_response()
        }
        Err(r) => r.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn message(kind: u8, encoding: u8, body: &[u8]) -> Vec<u8> {
        // Version 3 header: version, encoding, type, subscription, org id (i32), timestamp (i64).
        let mut out = vec![3, encoding, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn decodes_manifests() {
        let pod = br#"{"metadata":{"name":"storefront-7d9","namespace":"shop","uid":"u1"},"spec":{},"status":{"phase":"Running"}}"#;
        let m = pb::CollectorManifest {
            cluster_name: "prod-eu".into(),
            manifests: vec![
                pb::Manifest { r#type: 41, uid: "u1".into(), content: pod.to_vec(), content_type: "json".into(), api_version: "v1".into(), ..Default::default() },
                pb::Manifest { r#type: 41, uid: "u2".into(), content: pod.to_vec(), is_terminated: true, ..Default::default() },
            ],
            ..Default::default()
        };
        let zipped = zstd::encode_all(m.encode_to_vec().as_slice(), 3).unwrap();
        let back = decode(&message(TYPE_MANIFEST, 2, &zipped)).unwrap();
        let (v, n) = to_otlp(&back);
        assert_eq!(n, 1, "terminated objects are skipped");
        let rl = &v.resource_logs[0];
        let Some(otlp::any_value::Value::StringValue(text)) = rl.scope_logs[0].log_records[0].body.as_ref().and_then(|b| b.value.clone()) else { panic!("string body") };
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["kind"], "Pod");
        assert_eq!(body["apiVersion"], "v1");
        assert_eq!(body["metadata"]["uid"], "u1");
        let attrs = &rl.resource.as_ref().unwrap().attributes;
        assert_eq!(attrs[0], otlp::kv("k8s.cluster.name", "prod-eu"));
        assert_eq!(attrs[1], otlp::kv("oc.signal", "k8s.objects"));

        let crd = pb::Wrapped { manifest: Some(m) };
        assert_eq!(decode(&message(TYPE_MANIFEST_CRD, 0, &crd.encode_to_vec())).unwrap().cluster_name, "prod-eu");
        assert!(decode(&message(41, 0, b"")).is_err());
        assert!(read_header(&[9, 0, 0, 0, 0]).is_err());
    }
}
