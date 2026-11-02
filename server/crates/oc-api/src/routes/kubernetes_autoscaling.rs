//! Kubernetes → Autoscaling: rightsizing recommendations, idle cost, the autoscalers running in each
//! cluster and the ones left misconfigured, cluster scaling and autoscaling profiles.
//!
//! The Datadog Cluster Agent does the scaling (`DatadogPodAutoscaler` objects, generated from
//! `DatadogPodAutoscalerClusterProfile`s for workloads labelled `autoscaling.datadoghq.com/profile`).
//! Its orchestrator check sends those objects and Karpenter's NodePools with the rest of the
//! inventory, so this module only reads:
//! - the explorer's inventory (`kubernetes::resources`): workloads, pods with their containers'
//!   requests, nodes, HPAs and CRDs;
//! - the autoscaling objects (`otel.k8s_inventory`, whole `spec` and `status`);
//! - 24 hours of container usage (the Datadog Agent's kubelet check or kubeletstats), mapped to
//!   workloads by pod name (`<deployment>-<hash>-<id>`, `<statefulset>-<ordinal>`), so pods
//!   replaced during the day still count.
//!
//! Costs use the org's prices per vCPU-hour and GiB-hour (Settings), over months of 730 hours:
//! idle is requests minus usage; cluster idle is allocatable capacity minus requests.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::kubernetes::{resources, KubeKind, KubeResource};
use super::metrics::{name_list, now_ms, points_subquery, with_window, OPEN_END_MS};
use crate::auth::Admin;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};
use crate::ttl_cache::TtlCache;

pub const HOURS_PER_MONTH: f64 = 730.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const MIB: f64 = 1024.0 * 1024.0;
/// Recommended requests: the 95th percentile of CPU and the peak of memory, plus this headroom.
const HEADROOM: f64 = 1.15;
const MIN_CPU: f64 = 0.01;
const MIN_MEMORY: f64 = 32.0 * MIB;
/// Less usage history than this and a recommendation would be a guess.
pub const MIN_HOURS: f64 = 6.0;
/// The label that makes the Cluster Agent create an autoscaler for a workload from a cluster profile.
pub const PROFILE_LABEL: &str = "autoscaling.datadoghq.com/profile";
/// The annotation pointing a DatadogPodAutoscaler at an external recommender.
pub const RECOMMENDER_ANNOTATION: &str = "autoscaling.datadoghq.com/custom-recommender";
/// The label of the profiles the Cluster Agent installs itself (value `datadog`).
const BUILTIN_LABEL: &str = "autoscaling.datadoghq.com/managed-by";

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingSettings {
    /// USD per vCPU-hour of requests.
    pub cpu_hour_price: f64,
    /// USD per GiB-hour of requests.
    pub memory_gib_hour_price: f64,
}

impl Default for AutoscalingSettings {
    fn default() -> Self {
        AutoscalingSettings { cpu_hour_price: 0.0316, memory_gib_hour_price: 0.0042 }
    }
}

impl AutoscalingSettings {
    /// Monthly cost of cores and bytes.
    pub fn monthly(&self, cpu: f64, memory: f64) -> f64 {
        (cpu * self.cpu_hour_price + memory / GIB * self.memory_gib_hour_price) * HOURS_PER_MONTH
    }
}

/// One container's usage over the last 24 hours (cores, bytes), over all pods of its workload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerUsage {
    pub cpu_p95: f64,
    pub cpu_avg: f64,
    pub memory_max: f64,
    pub memory_avg: f64,
    /// Hours between the first and the last point.
    pub hours: f64,
}

/// (cluster, namespace, workload name) → container → usage.
pub type WorkloadUsage = HashMap<(String, String, String), HashMap<String, ContainerUsage>>;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContainerRecommendation {
    pub name: String,
    pub cpu_request: Option<f64>,
    pub memory_request: Option<f64>,
    pub cpu_p95: Option<f64>,
    pub cpu_avg: Option<f64>,
    pub memory_max: Option<f64>,
    pub memory_avg: Option<f64>,
    pub recommended_cpu: Option<f64>,
    pub recommended_memory: Option<f64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadRecommendation {
    pub uid: String,
    pub cluster: String,
    pub namespace: String,
    /// Deployment or StatefulSet.
    pub kind: String,
    pub name: String,
    pub service: Option<String>,
    pub replicas: u32,
    pub containers: Vec<ContainerRecommendation>,
    /// Requested minus used, over all replicas (cores, bytes).
    pub cpu_idle: f64,
    pub memory_unused: f64,
    /// Monthly costs: of what the replicas request, of what they use, and of the idle part.
    pub requests_cost: f64,
    pub usage_cost: f64,
    pub idle_cost: f64,
    /// What the recommended requests would save per month (0 when they'd cost more).
    pub projected_savings: f64,
    /// Why the workload can't be autoscaled yet (empty: ready).
    pub not_ready: Vec<String>,
    /// The Cluster Agent's workload autoscaling is installed in the cluster.
    pub agent_setup: bool,
    /// The autoscaler targeting the workload, if any (`Kind/name`).
    pub autoscaler: Option<String>,
    /// The cluster profile its label asks for.
    pub profile: Option<String>,
}

/// An object of a kind the explorer doesn't model (DatadogPodAutoscaler, cluster profiles,
/// Karpenter NodePools): its metadata and `spec` / `status`.
#[derive(Clone, Debug, Default)]
pub struct CustomObject {
    pub cluster: String,
    pub kind: String,
    pub uid: String,
    pub namespace: String,
    pub name: String,
    pub created: Option<i64>,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub owner_kind: String,
    pub owner_name: String,
    pub doc: Value,
}

fn up_to(v: f64, step: f64) -> f64 {
    ((v / step) - 1e-6).ceil() * step
}

/// Recommended (cores, bytes) for a container's usage.
fn recommend(u: &ContainerUsage) -> (f64, f64) {
    let cpu = (up_to(u.cpu_p95 * HEADROOM, 0.001) * 1000.0).round() / 1000.0;
    (cpu.max(MIN_CPU), up_to(u.memory_max * HEADROOM, MIB).max(MIN_MEMORY))
}

fn tag<'a>(r: &'a KubeResource, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    r.tags.iter().find_map(|t| t.strip_prefix(&prefix))
}

/// The tag pods carry for their workload of `kind`.
fn pod_tag(kind: KubeKind) -> &'static str {
    if kind == KubeKind::StatefulSets {
        "kube_stateful_set"
    } else {
        "kube_deployment"
    }
}

fn api_kind(kind: KubeKind) -> &'static str {
    if kind == KubeKind::StatefulSets {
        "StatefulSet"
    } else {
        "Deployment"
    }
}

/// The pods of a workload, by its pods' tags.
fn pods_of<'a>(rows: &'a [KubeResource], w: &KubeResource) -> impl Iterator<Item = &'a KubeResource> {
    let (key, cluster, namespace, name) = (pod_tag(w.kind.unwrap_or(KubeKind::Deployments)), w.cluster.clone(), w.namespace.clone(), w.name.clone());
    rows.iter().filter(move |p| p.kind == Some(KubeKind::Pods) && p.cluster == cluster && p.namespace == namespace && tag(p, key) == Some(name.as_str()))
}

/// Recommendations for every Deployment and StatefulSet.
pub fn recommendations(settings: &AutoscalingSettings, rows: &[KubeResource], usage: &WorkloadUsage, autoscalers: &[KubeAutoscaler]) -> Vec<WorkloadRecommendation> {
    let dpa_clusters: HashSet<&str> = rows
        .iter()
        .filter(|r| r.kind == Some(KubeKind::CustomResourceDefinitions) && r.name == super::kubernetes_integration::DPA_CRD)
        .map(|r| r.cluster.as_str())
        .collect();
    rows.iter()
        .filter(|r| matches!(r.kind, Some(KubeKind::Deployments | KubeKind::StatefulSets)))
        .map(|w| {
            let namespace = w.namespace.clone().unwrap_or_default();
            let kind = api_kind(w.kind.unwrap_or(KubeKind::Deployments));
            let replicas = w.desired.unwrap_or(0);
            // A current pod's containers and their requests (all pods of a workload share its template).
            let template = pods_of(rows, w).find(|p| !p.containers.is_empty());
            let used = usage.get(&(w.cluster.clone(), namespace.clone(), w.name.clone()));
            let mut not_ready = Vec::new();
            let mut containers = Vec::new();
            let (mut cpu_idle, mut memory_unused, mut requests_cost, mut usage_cost, mut savings) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for c in template.map(|p| p.containers.iter().filter(|c| !c.init).collect::<Vec<_>>()).unwrap_or_default() {
                let u = used.and_then(|m| m.get(&c.name));
                let (cpu_req, mem_req) = (c.cpu.requests, c.memory.requests);
                let rec = u.map(recommend);
                if cpu_req.is_none() {
                    not_ready.push(format!("No CPU request on container {}", c.name));
                }
                if mem_req.is_none() {
                    not_ready.push(format!("No memory request on container {}", c.name));
                }
                let r = f64::from(replicas);
                if let Some(u) = u {
                    cpu_idle += r * (cpu_req.unwrap_or(0.0) - u.cpu_avg).max(0.0);
                    memory_unused += r * (mem_req.unwrap_or(0.0) - u.memory_avg).max(0.0);
                    usage_cost += r * settings.monthly(u.cpu_avg, u.memory_avg);
                }
                requests_cost += r * settings.monthly(cpu_req.unwrap_or(0.0), mem_req.unwrap_or(0.0));
                if let (Some((rc, rm)), Some(cr), Some(mr)) = (rec, cpu_req, mem_req) {
                    savings += r * (settings.monthly(cr, mr) - settings.monthly(rc, rm));
                }
                containers.push(ContainerRecommendation {
                    name: c.name.clone(),
                    cpu_request: cpu_req,
                    memory_request: mem_req,
                    cpu_p95: u.map(|u| u.cpu_p95),
                    cpu_avg: u.map(|u| u.cpu_avg),
                    memory_max: u.map(|u| u.memory_max),
                    memory_avg: u.map(|u| u.memory_avg),
                    recommended_cpu: rec.map(|r| r.0),
                    recommended_memory: rec.map(|r| r.1),
                });
            }
            let hours = used.map(|m| m.values().map(|u| u.hours).fold(0.0, f64::max));
            match hours {
                None => not_ready.push("No usage metrics".into()),
                Some(h) if h < MIN_HOURS => not_ready.push(format!("Less than {MIN_HOURS} hours of usage")),
                _ => {}
            }
            let scaler = autoscalers.iter().find(|a| a.cluster == w.cluster && a.namespace == namespace && a.target_kind == kind && a.target_name == w.name);
            if let Some(a) = scaler {
                not_ready.push(format!("Scaled by {} {}", a.kind, a.name));
            }
            WorkloadRecommendation {
                uid: w.uid.clone(),
                cluster: w.cluster.clone(),
                namespace,
                kind: kind.into(),
                name: w.name.clone(),
                service: tag(w, "service").map(str::to_string),
                replicas,
                containers,
                cpu_idle,
                memory_unused,
                requests_cost,
                usage_cost,
                idle_cost: settings.monthly(cpu_idle, memory_unused),
                projected_savings: savings.max(0.0),
                not_ready,
                agent_setup: dpa_clusters.contains(w.cluster.as_str()),
                autoscaler: scaler.map(|a| format!("{}/{}", a.kind, a.name)),
                profile: w.labels.get(PROFILE_LABEL).cloned(),
            }
        })
        .collect()
}

/// The workload a pod belonged to, from its name: `<deployment>-<pod-template-hash>-<suffix>` or
/// `<statefulset>-<ordinal>`. `workloads` are the (kind, name) of one namespace; the longest match wins.
pub fn workload_of_pod<'a>(pod: &str, workloads: &'a [(String, String)]) -> Option<&'a (String, String)> {
    let alnum = |s: &str, min: usize, max: usize| (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    workloads
        .iter()
        .filter(|(kind, name)| {
            let Some(rest) = pod.strip_prefix(name.as_str()).and_then(|r| r.strip_prefix('-')) else { return false };
            if kind == "StatefulSet" {
                !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
            } else {
                matches!(rest.split_once('-'), Some((hash, id)) if alnum(hash, 1, 10) && alnum(id, 5, 5))
            }
        })
        .max_by_key(|(_, name)| name.len())
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KubeAutoscaler {
    pub uid: String,
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    /// DatadogPodAutoscaler or HorizontalPodAutoscaler.
    pub kind: String,
    /// DatadogPodAutoscaler: Local (created in the cluster) or Remote.
    pub owner: Option<String>,
    /// The cluster profile it was generated from.
    pub profile: Option<String>,
    pub created_at: Option<i64>,
    pub target_kind: String,
    pub target_name: String,
    pub target_exists: bool,
    pub horizontal: bool,
    pub vertical: bool,
    pub min_replicas: Option<u32>,
    pub max_replicas: Option<u32>,
    pub current_replicas: Option<u32>,
    pub desired_replicas: Option<u32>,
    pub pods_running: u32,
    pub pods_pending: u32,
    pub pods_failing: u32,
    /// The external recommender endpoint of its `custom-recommender` annotation.
    pub recommender: Option<String>,
    /// Why it doesn't work: missing target, error conditions.
    pub problems: Vec<String>,
}

fn text<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_str()
}

fn count(v: &Value, path: &[&str]) -> Option<u32> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// The DatadogPodAutoscalers and HPAs of every cluster, with their target's pods.
pub fn autoscalers(rows: &[KubeResource], dpas: &[CustomObject]) -> Vec<KubeAutoscaler> {
    let workload = |cluster: &str, namespace: &str, kind: &str, name: &str| {
        rows.iter().find(|r| {
            r.cluster == cluster && r.namespace.as_deref() == Some(namespace) && r.name == name && matches!((kind, r.kind), ("Deployment", Some(KubeKind::Deployments)) | ("StatefulSet", Some(KubeKind::StatefulSets)))
        })
    };
    let with_target = |mut a: KubeAutoscaler| {
        match workload(&a.cluster, &a.namespace, &a.target_kind, &a.target_name) {
            Some(w) => {
                a.target_exists = true;
                for p in pods_of(rows, w) {
                    match p.status.as_str() {
                        "Running" => a.pods_running += 1,
                        "Pending" | "ContainerCreating" => a.pods_pending += 1,
                        "Succeeded" | "Completed" => {}
                        _ => a.pods_failing += 1,
                    }
                }
            }
            None => a.problems.insert(0, format!("Target {} {} not found", a.target_kind, a.target_name)),
        }
        a
    };
    let mut out: Vec<KubeAutoscaler> = dpas
        .iter()
        .filter(|o| o.kind == "DatadogPodAutoscaler")
        .map(|o| {
            let spec = o.doc.get("spec").cloned().unwrap_or(Value::Null);
            let status = o.doc.get("status").cloned().unwrap_or(Value::Null);
            let disabled = |dir: &str| text(&spec, &["applyPolicy", dir, "strategy"]) == Some("Disabled");
            let problems = status
                .get("conditions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("Error") && c.get("status").and_then(Value::as_str) == Some("True"))
                .filter_map(|c| c.get("message").and_then(Value::as_str).map(str::to_string))
                .collect();
            let recommender = o.annotations.get(RECOMMENDER_ANNOTATION).and_then(|a| serde_json::from_str::<Value>(a).ok()).and_then(|v| v.get("endpoint").and_then(Value::as_str).map(str::to_string));
            with_target(KubeAutoscaler {
                uid: o.uid.clone(),
                cluster: o.cluster.clone(),
                namespace: o.namespace.clone(),
                name: o.name.clone(),
                kind: "DatadogPodAutoscaler".into(),
                owner: text(&spec, &["owner"]).map(str::to_string),
                profile: if o.owner_kind == "DatadogPodAutoscalerClusterProfile" { Some(o.owner_name.clone()) } else { o.labels.get(PROFILE_LABEL).cloned() },
                created_at: o.created,
                target_kind: text(&spec, &["targetRef", "kind"]).unwrap_or_default().into(),
                target_name: text(&spec, &["targetRef", "name"]).unwrap_or_default().into(),
                target_exists: false,
                horizontal: !(disabled("scaleUp") && disabled("scaleDown")),
                vertical: text(&spec, &["applyPolicy", "update", "strategy"]) != Some("Disabled"),
                min_replicas: count(&spec, &["constraints", "minReplicas"]),
                max_replicas: count(&spec, &["constraints", "maxReplicas"]),
                current_replicas: count(&status, &["currentReplicas"]),
                desired_replicas: count(&status, &["horizontal", "target", "desiredReplicas"]),
                pods_running: 0,
                pods_pending: 0,
                pods_failing: 0,
                recommender,
                problems,
            })
        })
        .collect();
    for h in rows.iter().filter(|r| r.kind == Some(KubeKind::HorizontalPodAutoscalers)) {
        let (target_kind, target_name) = h.target.as_deref().and_then(|t| t.split_once('/')).unwrap_or_default();
        let (min, max) = h.replicas_range.as_deref().and_then(|r| r.split_once('-')).unwrap_or_default();
        out.push(with_target(KubeAutoscaler {
            uid: h.uid.clone(),
            cluster: h.cluster.clone(),
            namespace: h.namespace.clone().unwrap_or_default(),
            name: h.name.clone(),
            kind: "HorizontalPodAutoscaler".into(),
            owner: None,
            profile: None,
            created_at: h.created_at,
            target_kind: target_kind.into(),
            target_name: target_name.into(),
            target_exists: false,
            horizontal: true,
            vertical: false,
            min_replicas: min.parse().ok(),
            max_replicas: max.parse().ok(),
            current_replicas: h.current,
            desired_replicas: h.desired,
            pods_running: 0,
            pods_pending: 0,
            pods_failing: 0,
            recommender: None,
            problems: Vec::new(),
        }));
    }
    out
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeAutoscaler {
    Karpenter,
    ClusterAutoscaler,
    None,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterScaling {
    pub cluster: String,
    pub autoscaling: NodeAutoscaler,
    pub nodes: u32,
    pub cpu_capacity: f64,
    pub memory_capacity: f64,
    pub cpu_requests: f64,
    pub memory_requests: f64,
    /// Allocatable capacity no pod requests, per month.
    pub cluster_idle_cost: f64,
    /// Requested capacity no pod uses, per month.
    pub workload_idle_cost: f64,
    pub usage_cost: f64,
    /// What the workloads' recommendations would save per month.
    pub projected_workload_savings: f64,
    /// Idle capacity beyond the largest node (kept as headroom), per month: what removing nodes
    /// would save.
    pub projected_cluster_savings: f64,
}

pub fn cluster_scaling(settings: &AutoscalingSettings, rows: &[KubeResource], workloads: &[WorkloadRecommendation], node_pools: &[CustomObject]) -> Vec<ClusterScaling> {
    let mut out: BTreeMap<String, ClusterScaling> = BTreeMap::new();
    let mut largest: HashMap<String, (f64, f64)> = HashMap::new();
    let blank = |cluster: &str| ClusterScaling {
        cluster: cluster.to_string(),
        autoscaling: NodeAutoscaler::None,
        nodes: 0,
        cpu_capacity: 0.0,
        memory_capacity: 0.0,
        cpu_requests: 0.0,
        memory_requests: 0.0,
        cluster_idle_cost: 0.0,
        workload_idle_cost: 0.0,
        usage_cost: 0.0,
        projected_workload_savings: 0.0,
        projected_cluster_savings: 0.0,
    };
    for r in rows {
        match r.kind {
            Some(KubeKind::Nodes) => {
                let (cpu, mem) = (r.cpu.as_ref().and_then(|u| u.capacity).unwrap_or(0.0), r.memory.as_ref().and_then(|u| u.capacity).unwrap_or(0.0));
                let c = out.entry(r.cluster.clone()).or_insert_with(|| blank(&r.cluster));
                c.nodes += 1;
                c.cpu_capacity += cpu;
                c.memory_capacity += mem;
                let l = largest.entry(r.cluster.clone()).or_default();
                *l = (l.0.max(cpu), l.1.max(mem));
            }
            Some(KubeKind::Pods) if !matches!(r.status.as_str(), "Succeeded" | "Completed" | "Failed") => {
                let c = out.entry(r.cluster.clone()).or_insert_with(|| blank(&r.cluster));
                c.cpu_requests += r.cpu.as_ref().and_then(|u| u.requests).unwrap_or(0.0);
                c.memory_requests += r.memory.as_ref().and_then(|u| u.requests).unwrap_or(0.0);
            }
            Some(KubeKind::Deployments) if r.name.contains("cluster-autoscaler") => {
                let c = out.entry(r.cluster.clone()).or_insert_with(|| blank(&r.cluster));
                if c.autoscaling == NodeAutoscaler::None {
                    c.autoscaling = NodeAutoscaler::ClusterAutoscaler;
                }
            }
            _ => {}
        }
    }
    for p in node_pools.iter().filter(|p| p.kind == "NodePool") {
        out.entry(p.cluster.clone()).or_insert_with(|| blank(&p.cluster)).autoscaling = NodeAutoscaler::Karpenter;
    }
    for w in workloads {
        let c = out.entry(w.cluster.clone()).or_insert_with(|| blank(&w.cluster));
        c.workload_idle_cost += w.idle_cost;
        c.usage_cost += w.usage_cost;
        c.projected_workload_savings += w.projected_savings;
    }
    for c in out.values_mut() {
        let (idle_cpu, idle_mem) = ((c.cpu_capacity - c.cpu_requests).max(0.0), (c.memory_capacity - c.memory_requests).max(0.0));
        c.cluster_idle_cost = settings.monthly(idle_cpu, idle_mem);
        let (node_cpu, node_mem) = largest.get(&c.cluster).copied().unwrap_or_default();
        c.projected_cluster_savings = settings.monthly((idle_cpu - node_cpu).max(0.0), (idle_mem - node_mem).max(0.0));
    }
    out.into_values().collect()
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    /// Installed by the Cluster Agent itself (`datadog-optimize-*`).
    Builtin,
    /// A DatadogPodAutoscalerClusterProfile found in a cluster.
    Cluster,
    /// Saved in ObserveCraft, not seen in a cluster yet.
    Saved,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingProfile {
    pub name: String,
    pub source: ProfileSource,
    /// The id of the copy saved in ObserveCraft, if any.
    pub id: Option<String>,
    /// Clusters where the profile is deployed.
    pub clusters: Vec<String>,
    /// Workloads labelled with it.
    pub workloads: u32,
    /// `spec.template` of the profile.
    #[schema(value_type = Object)]
    pub template: Value,
}

/// A profile saved in ObserveCraft: (id, name, template).
pub type SavedProfile = (String, String, Value);

/// Profiles seen in clusters, merged by name with those saved in ObserveCraft.
pub fn profiles(rows: &[KubeResource], objects: &[CustomObject], saved: &[SavedProfile]) -> Vec<AutoscalingProfile> {
    let mut out: BTreeMap<String, AutoscalingProfile> = BTreeMap::new();
    for o in objects.iter().filter(|o| o.kind == "DatadogPodAutoscalerClusterProfile") {
        let p = out.entry(o.name.clone()).or_insert_with(|| AutoscalingProfile {
            name: o.name.clone(),
            source: if o.labels.get(BUILTIN_LABEL).map(String::as_str) == Some("datadog") { ProfileSource::Builtin } else { ProfileSource::Cluster },
            id: None,
            clusters: Vec::new(),
            workloads: 0,
            template: o.doc.pointer("/spec/template").cloned().unwrap_or(Value::Null),
        });
        if !p.clusters.contains(&o.cluster) {
            p.clusters.push(o.cluster.clone());
        }
    }
    for (id, name, template) in saved {
        let p = out.entry(name.clone()).or_insert_with(|| AutoscalingProfile { name: name.clone(), source: ProfileSource::Saved, id: None, clusters: Vec::new(), workloads: 0, template: template.clone() });
        p.id = Some(id.clone());
    }
    for r in rows.iter().filter(|r| matches!(r.kind, Some(KubeKind::Deployments | KubeKind::StatefulSets))) {
        if let Some(p) = r.labels.get(PROFILE_LABEL).and_then(|name| out.get_mut(name)) {
            p.workloads += 1;
        }
    }
    for p in out.values_mut() {
        p.clusters.sort();
    }
    out.into_values().collect()
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingSummary {
    /// Monthly costs.
    pub usage_cost: f64,
    pub workload_idle_cost: f64,
    pub cluster_idle_cost: f64,
    pub workload_savings: f64,
    pub cluster_savings: f64,
    pub workloads: u32,
    /// Workloads no autoscaler targets.
    pub unscaled_workloads: u32,
    pub clusters: u32,
    /// Clusters without a node autoscaler.
    pub unscaled_clusters: u32,
}

pub fn summary(clusters: &[ClusterScaling], workloads: &[WorkloadRecommendation]) -> AutoscalingSummary {
    let n = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
    AutoscalingSummary {
        usage_cost: clusters.iter().map(|c| c.usage_cost).sum(),
        workload_idle_cost: clusters.iter().map(|c| c.workload_idle_cost).sum(),
        cluster_idle_cost: clusters.iter().map(|c| c.cluster_idle_cost).sum(),
        workload_savings: clusters.iter().map(|c| c.projected_workload_savings).sum(),
        cluster_savings: clusters.iter().map(|c| c.projected_cluster_savings).sum(),
        workloads: n(workloads.len()),
        unscaled_workloads: n(workloads.iter().filter(|w| w.autoscaler.is_none()).count()),
        clusters: n(clusters.len()),
        unscaled_clusters: n(clusters.iter().filter(|c| c.autoscaling == NodeAutoscaler::None).count()),
    }
}

// ================================================================ loading

/// Kinds read for autoscaling that the explorer doesn't model.
const CUSTOM_KINDS: &[&str] = &["DatadogPodAutoscaler", "DatadogPodAutoscalerClusterProfile", "NodePool"];
/// Container usage metrics: kubeletstats (cores, bytes), the Datadog Agent's container check (the same
/// `container.*` names, CPU in nanocores, marked `observecraft.collector = datadog-agent`) and its
/// older kubelet check (`kubernetes.*`, CPU in nanocores).
pub(crate) const USAGE_METRICS: &[&str] = &["container.cpu.usage", "container.cpu.utilization", "container.memory.working_set", "kubernetes.cpu.usage.total", "kubernetes.memory.working_set"];
/// A usage point's CPU in cores (only meaningful for CPU metrics).
pub(crate) const CPU_CORES: &str = "if(MetricName = 'kubernetes.cpu.usage.total' OR ResourceAttributes['observecraft.collector'] = 'datadog-agent', Value / 1e9, Value)";
pub(crate) const IS_CPU: &str = "MetricName IN ('container.cpu.usage', 'container.cpu.utilization', 'kubernetes.cpu.usage.total')";
pub(crate) const IS_MEMORY: &str = "MetricName IN ('container.memory.working_set', 'kubernetes.memory.working_set')";
/// The 24-hour usage query is reused this long per org.
const USAGE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// The current objects of `kinds` with their metadata and inventory document (`spec`, `status`).
pub(crate) async fn custom_objects(st: &AppState, org: &str, kinds: &[&str]) -> Result<Vec<CustomObject>, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Obj {
        cluster: String,
        kind: String,
        uid: String,
        namespace: String,
        name: String,
        created: Option<i64>,
        label_keys: Vec<String>,
        label_values: Vec<String>,
        annotation_keys: Vec<String>,
        annotation_values: Vec<String>,
        owner_kind: String,
        owner_name: String,
        doc: String,
    }
    let rows = st
        .ch(org)
        .query(
            "SELECT Cluster AS cluster, Kind AS kind, Uid AS uid, Namespace AS namespace, Name AS name, toInt64(toUnixTimestamp(Created)) * 1000 AS created, \
             mapKeys(Labels) AS label_keys, mapValues(Labels) AS label_values, mapKeys(Annotations) AS annotation_keys, mapValues(Annotations) AS annotation_values, \
             OwnerKind AS owner_kind, OwnerName AS owner_name, Doc AS doc \
             FROM otel.k8s_inventory FINAL WHERE OrgId = {oc_org:String} AND Kind IN ? AND NOT Deleting AND LastSeen > now64(3) - INTERVAL 3 MINUTE LIMIT 20000",
        )
        .bind(kinds)
        .fetch_all::<Obj>()
        .await?;
    Ok(rows
        .into_iter()
        .map(|o| CustomObject {
            cluster: o.cluster,
            kind: o.kind,
            uid: o.uid,
            namespace: o.namespace,
            name: o.name,
            created: o.created,
            labels: o.label_keys.into_iter().zip(o.label_values).collect(),
            annotations: o.annotation_keys.into_iter().zip(o.annotation_values).collect(),
            owner_kind: o.owner_kind,
            owner_name: o.owner_name,
            doc: serde_json::from_str(&o.doc).unwrap_or(Value::Null),
        })
        .collect())
}

#[derive(clickhouse::Row, Deserialize)]
struct PodContainerUsage {
    cluster: String,
    namespace: String,
    pod: String,
    container: String,
    cpu_p95: f64,
    cpu_avg: f64,
    mem_max: f64,
    mem_avg: f64,
    hours: f64,
}

/// 24 hours of usage per (pod, container) in 5-minute steps, from either collector.
async fn pod_usage(st: &AppState, org: &str) -> Result<Vec<PodContainerUsage>, ApiError> {
    let now = now_ms();
    let src = points_subquery("gauge", USAGE_METRICS);
    let rows = with_window(st.ch(org), now - 24 * 3_600_000, now + OPEN_END_MS)
        .query(&format!(
            "SELECT cluster, namespace, pod, container, ifNotFinite(quantileIf(0.95)(cpu, cpu >= 0), 0) AS cpu_p95, ifNotFinite(avgIf(cpu, cpu >= 0), 0) AS cpu_avg, \
             ifNotFinite(maxIf(mem, mem >= 0), 0) AS mem_max, ifNotFinite(avgIf(mem, mem >= 0), 0) AS mem_avg, toFloat64(dateDiff('second', min(b), max(b)) + 300) / 3600 AS hours \
             FROM (SELECT ResourceAttributes['k8s.cluster.name'] AS cluster, ResourceAttributes['k8s.namespace.name'] AS namespace, ResourceAttributes['k8s.pod.name'] AS pod, \
                   ResourceAttributes['k8s.container.name'] AS container, toStartOfFiveMinutes(TimeUnix) AS b, \
                   ifNotFinite(avgIf({CPU_CORES}, {IS_CPU}), -1) AS cpu, ifNotFinite(avgIf(Value, {IS_MEMORY}), -1) AS mem \
                   FROM {src} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 24 HOUR AND MetricName IN ({names}) AND pod != '' AND container != '' \
                   GROUP BY cluster, namespace, pod, container, b) \
             GROUP BY cluster, namespace, pod, container LIMIT 200000",
            names = name_list(USAGE_METRICS),
        ))
        .fetch_all::<PodContainerUsage>()
        .await?;
    Ok(rows)
}

/// (cluster, namespace, pod, container, usage).
pub type PodUsage = (String, String, String, String, ContainerUsage);
/// A container's usage summed over pods, and how many.
type Summed = (ContainerUsage, f64);

/// Pod usage summed up per workload container: the highest p95 and peak of its pods, the mean of
/// their averages.
pub fn workload_usage(rows: &[KubeResource], pods: impl IntoIterator<Item = PodUsage>) -> WorkloadUsage {
    let mut workloads: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for r in rows.iter().filter(|r| matches!(r.kind, Some(KubeKind::Deployments | KubeKind::StatefulSets))) {
        workloads.entry((r.cluster.clone(), r.namespace.clone().unwrap_or_default())).or_default().push((api_kind(r.kind.unwrap_or(KubeKind::Deployments)).to_string(), r.name.clone()));
    }
    // Per workload container: the running aggregate and the number of pods in it.
    let mut sums: HashMap<(String, String, String), HashMap<String, Summed>> = HashMap::new();
    for (cluster, namespace, pod, container, u) in pods {
        let Some((_, name)) = workloads.get(&(cluster.clone(), namespace.clone())).and_then(|ws| workload_of_pod(&pod, ws)) else { continue };
        let (acc, n) = sums.entry((cluster, namespace, name.clone())).or_default().entry(container).or_default();
        acc.cpu_p95 = acc.cpu_p95.max(u.cpu_p95);
        acc.memory_max = acc.memory_max.max(u.memory_max);
        acc.hours = acc.hours.max(u.hours);
        acc.cpu_avg += u.cpu_avg;
        acc.memory_avg += u.memory_avg;
        *n += 1.0;
    }
    sums.into_iter()
        .map(|(k, containers)| {
            let containers = containers.into_iter().map(|(c, (u, n))| (c, ContainerUsage { cpu_avg: u.cpu_avg / n, memory_avg: u.memory_avg / n, ..u })).collect();
            (k, containers)
        })
        .collect()
}

async fn usage_of(st: &AppState, org: &str, rows: &[KubeResource]) -> Result<WorkloadUsage, ApiError> {
    static CACHE: std::sync::LazyLock<TtlCache<Vec<PodUsage>>> = std::sync::LazyLock::new(|| TtlCache::new(USAGE_TTL).with_max_entries(1000));
    let pods = CACHE
        .get_or_load(org, || async {
            Ok::<_, ApiError>(
                pod_usage(st, org)
                    .await?
                    .into_iter()
                    .map(|p| (p.cluster, p.namespace, p.pod, p.container, ContainerUsage { cpu_p95: p.cpu_p95, cpu_avg: p.cpu_avg, memory_max: p.mem_max, memory_avg: p.mem_avg, hours: p.hours }))
                    .collect(),
            )
        })
        .await?;
    Ok(workload_usage(rows, pods.iter().cloned()))
}

pub async fn load_settings(st: &AppState, org: &str) -> Result<AutoscalingSettings, sqlx::Error> {
    let row: Option<(f64, f64)> = sqlx::query_as("SELECT cpu_hour_price, memory_gib_hour_price FROM kube_autoscaling_settings WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row.map(|(cpu_hour_price, memory_gib_hour_price)| AutoscalingSettings { cpu_hour_price, memory_gib_hour_price }).unwrap_or_default())
}

async fn saved_profiles(st: &AppState, org: &str) -> Result<Vec<SavedProfile>, sqlx::Error> {
    let rows: Vec<(String, String, sqlx::types::Json<Value>)> = sqlx::query_as("SELECT id::text, name, template FROM kube_autoscaling_profiles WHERE org_id = $1 ORDER BY name").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|(id, name, t)| (id, name, t.0)).collect())
}

// ================================================================ routes

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingOverview {
    pub settings: AutoscalingSettings,
    pub summary: AutoscalingSummary,
    pub clusters: Vec<ClusterScaling>,
    pub workloads: Vec<WorkloadRecommendation>,
    pub autoscalers: Vec<KubeAutoscaler>,
    pub profiles: Vec<AutoscalingProfile>,
    /// The external recommender's last answers (last day), per autoscaler target.
    pub recommendations: Vec<super::kubernetes_recommender::StoredRecommendation>,
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::Internal(e.into())
}

/// Everything the Autoscaling pages show; the UI filters by cluster and namespace.
#[utoipa::path(get, path = "/api/v1/kubernetes/autoscaling", operation_id = "get_kubernetes_autoscaling", responses((status = 200, body = AutoscalingOverview)), tag = "orchestration")]
pub async fn overview(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<AutoscalingOverview> {
    let rows = resources(&st, &org).await?;
    let (objects, settings, saved, answers) = tokio::try_join!(
        custom_objects(&st, &org, CUSTOM_KINDS),
        async { load_settings(&st, &org).await.map_err(internal) },
        async { saved_profiles(&st, &org).await.map_err(internal) },
        async { super::kubernetes_recommender::stored(&st, &org).await.map_err(internal) }
    )?;
    let usage = usage_of(&st, &org, &rows).await?;
    let autoscalers = autoscalers(&rows, &objects);
    let workloads = recommendations(&settings, &rows, &usage, &autoscalers);
    let clusters = cluster_scaling(&settings, &rows, &workloads, &objects);
    Ok(Json(AutoscalingOverview { summary: summary(&clusters, &workloads), profiles: profiles(&rows, &objects, &saved), settings, clusters, workloads, autoscalers, recommendations: answers }))
}

#[utoipa::path(put, path = "/api/v1/kubernetes/autoscaling/settings", operation_id = "update_kubernetes_autoscaling_settings", request_body = AutoscalingSettings, responses((status = 200, body = AutoscalingSettings), (status = 400)), tag = "orchestration")]
pub async fn update_settings(State(st): State<AppState>, Admin(ctx): Admin, Json(body): Json<AutoscalingSettings>) -> ApiResult<AutoscalingSettings> {
    let valid = |p: f64| p.is_finite() && (0.0..1000.0).contains(&p);
    if !valid(body.cpu_hour_price) || !valid(body.memory_gib_hour_price) {
        return Err(ApiError::BadRequest("prices must be between 0 and 1000 USD per hour".into()));
    }
    sqlx::query(
        "INSERT INTO kube_autoscaling_settings (org_id, cpu_hour_price, memory_gib_hour_price) VALUES ($1, $2, $3) \
         ON CONFLICT (org_id) DO UPDATE SET cpu_hour_price = EXCLUDED.cpu_hour_price, memory_gib_hour_price = EXCLUDED.memory_gib_hour_price, modified_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(body.cpu_hour_price)
    .bind(body.memory_gib_hour_price)
    .execute(&st.postgres)
    .await
    .map_err(internal)?;
    Ok(Json(body))
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ProfileInput {
    /// A Kubernetes object name (DNS subdomain).
    pub name: String,
    /// `spec.template` of the DatadogPodAutoscalerClusterProfile.
    #[schema(value_type = Object)]
    pub template: Value,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SavedProfileView {
    pub id: String,
    pub name: String,
    #[schema(value_type = Object)]
    pub template: Value,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !part.starts_with('-') && !part.ends_with('-'))
}

fn conflict_or_internal(e: sqlx::Error) -> ApiError {
    if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") {
        ApiError::Conflict("a profile with this name already exists".into())
    } else {
        internal(e)
    }
}

fn check_profile(p: &ProfileInput) -> Result<(), ApiError> {
    if !valid_name(&p.name) {
        return Err(ApiError::BadRequest("the name must be lowercase letters, digits, '-' and '.'".into()));
    }
    if !p.template.is_object() {
        return Err(ApiError::BadRequest("the template must be an object".into()));
    }
    Ok(())
}

#[utoipa::path(post, path = "/api/v1/kubernetes/autoscaling/profiles", operation_id = "create_kubernetes_autoscaling_profile", request_body = ProfileInput, responses((status = 200, body = SavedProfileView), (status = 400), (status = 409)), tag = "orchestration")]
pub async fn create_profile(State(st): State<AppState>, Admin(ctx): Admin, Json(body): Json<ProfileInput>) -> ApiResult<SavedProfileView> {
    check_profile(&body)?;
    let (id,): (String,) = sqlx::query_as("INSERT INTO kube_autoscaling_profiles (org_id, name, template) VALUES ($1, $2, $3) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&body.name)
        .bind(sqlx::types::Json(&body.template))
        .fetch_one(&st.postgres)
        .await
        .map_err(conflict_or_internal)?;
    Ok(Json(SavedProfileView { id, name: body.name, template: body.template }))
}

/// A saved profile's id, or 404 for anything that isn't a UUID.
fn profile_id(id: &str) -> Result<&str, ApiError> {
    let hex = id.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() });
    if id.len() == 36 && hex {
        Ok(id)
    } else {
        Err(ApiError::NotFound("profile not found".into()))
    }
}

#[utoipa::path(put, path = "/api/v1/kubernetes/autoscaling/profiles/{id}", operation_id = "update_kubernetes_autoscaling_profile", params(("id" = String, Path)), request_body = ProfileInput, responses((status = 200, body = SavedProfileView), (status = 400), (status = 404), (status = 409)), tag = "orchestration")]
pub async fn update_profile(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(body): Json<ProfileInput>) -> ApiResult<SavedProfileView> {
    check_profile(&body)?;
    let id = profile_id(&id)?;
    let updated = sqlx::query("UPDATE kube_autoscaling_profiles SET name = $3, template = $4, modified_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(id)
        .bind(&body.name)
        .bind(sqlx::types::Json(&body.template))
        .execute(&st.postgres)
        .await
        .map_err(conflict_or_internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound("profile not found".into()));
    }
    Ok(Json(SavedProfileView { id: id.to_string(), name: body.name, template: body.template }))
}

#[utoipa::path(delete, path = "/api/v1/kubernetes/autoscaling/profiles/{id}", operation_id = "delete_kubernetes_autoscaling_profile", params(("id" = String, Path)), responses((status = 204), (status = 404)), tag = "orchestration")]
pub async fn delete_profile(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let id = profile_id(&id)?;
    let deleted = sqlx::query("DELETE FROM kube_autoscaling_profiles WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(id).execute(&st.postgres).await.map_err(internal)?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound("profile not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::kubernetes::{KubeContainer, KubeUtilization};
    use serde_json::json;

    const S: AutoscalingSettings = AutoscalingSettings { cpu_hour_price: 0.04, memory_gib_hour_price: 0.005 };

    fn util(requests: Option<f64>) -> KubeUtilization {
        KubeUtilization { requests, ..Default::default() }
    }

    fn workload(kind: KubeKind, name: &str, replicas: u32) -> KubeResource {
        KubeResource {
            uid: format!("uid-{name}"),
            kind: Some(kind),
            cluster: "shop-prod".into(),
            namespace: Some("checkout".into()),
            name: name.into(),
            desired: Some(replicas),
            tags: vec![format!("service:{name}")],
            ..Default::default()
        }
    }

    fn pod(workload: &str, tag: &str, i: u32, cpu: Option<f64>, mem: Option<f64>, status: &str) -> KubeResource {
        KubeResource {
            uid: format!("pod-{workload}-{i}"),
            kind: Some(KubeKind::Pods),
            cluster: "shop-prod".into(),
            namespace: Some("checkout".into()),
            name: format!("{workload}-7d9f8c6b5-x{i}abc"),
            status: status.into(),
            tags: vec![format!("{tag}:{workload}")],
            containers: vec![KubeContainer {
                name: "app".into(),
                image: format!("registry.example.com/{workload}:2.4.0"),
                ready: true,
                restarts: 0,
                state: "running".into(),
                reason: None,
                started_at: None,
                init: false,
                cpu: util(cpu),
                memory: util(mem),
            }],
            ..Default::default()
        }
    }

    fn usage(entries: &[(&str, ContainerUsage)]) -> WorkloadUsage {
        entries.iter().map(|(w, u)| (("shop-prod".to_string(), "checkout".to_string(), w.to_string()), HashMap::from([("app".to_string(), u.clone())]))).collect()
    }

    #[test]
    fn pods_map_to_their_deployment_or_statefulset_by_name() {
        let ws = vec![("Deployment".to_string(), "order".to_string()), ("Deployment".to_string(), "order-service".to_string()), ("StatefulSet".to_string(), "orders-db".to_string())];
        assert_eq!(workload_of_pod("order-service-7d9f8c6b5-x2k9p", &ws).map(|w| w.1.as_str()), Some("order-service"));
        assert_eq!(workload_of_pod("order-6b5c7d9f8-abcde", &ws).map(|w| w.1.as_str()), Some("order"));
        assert_eq!(workload_of_pod("orders-db-0", &ws).map(|w| w.1.as_str()), Some("orders-db"));
        // A StatefulSet's pods have an ordinal, a Deployment's a hash and a suffix.
        assert_eq!(workload_of_pod("order-service-0", &ws), None);
        assert_eq!(workload_of_pod("billing-7d9f8c6b5-x2k9p", &ws), None);
    }

    #[test]
    fn requests_are_sized_to_the_p95_cpu_and_peak_memory_with_headroom() {
        let rows = vec![workload(KubeKind::Deployments, "storefront-api", 3), pod("storefront-api", "kube_deployment", 0, Some(1.0), Some(2.0 * GIB), "Running")];
        let u = usage(&[("storefront-api", ContainerUsage { cpu_p95: 0.2, cpu_avg: 0.1, memory_max: 512.0 * MIB, memory_avg: 400.0 * MIB, hours: 24.0 })]);
        let got = recommendations(&S, &rows, &u, &[]);
        assert_eq!(got.len(), 1);
        let w = &got[0];
        let c = &w.containers[0];
        assert_eq!((c.recommended_cpu, c.recommended_memory), (Some(0.23), Some(589.0 * MIB)));
        assert_eq!((w.kind.as_str(), w.replicas, w.service.as_deref()), ("Deployment", 3, Some("storefront-api")));
        // 3 replicas × (1 − 0.1) cores and (2 GiB − 400 MiB) idle.
        assert!((w.cpu_idle - 2.7).abs() < 1e-9);
        assert!((w.memory_unused - 3.0 * (2.0 * GIB - 400.0 * MIB)).abs() < 1.0);
        assert!((w.requests_cost - S.monthly(3.0, 6.0 * GIB)).abs() < 1e-6);
        assert!((w.idle_cost - S.monthly(w.cpu_idle, w.memory_unused)).abs() < 1e-6);
        assert!((w.projected_savings - 3.0 * (S.monthly(1.0, 2.0 * GIB) - S.monthly(0.23, 589.0 * MIB))).abs() < 1e-6);
        assert!(w.not_ready.is_empty(), "{:?}", w.not_ready);
    }

    #[test]
    fn workloads_without_requests_history_or_with_an_autoscaler_are_not_ready() {
        let rows = vec![
            workload(KubeKind::Deployments, "billing", 2),
            pod("billing", "kube_deployment", 0, None, Some(GIB), "Running"),
            workload(KubeKind::StatefulSets, "orders-db", 1),
            pod("orders-db", "kube_stateful_set", 0, Some(0.5), Some(GIB), "Running"),
            workload(KubeKind::Deployments, "api-gateway", 3),
            pod("api-gateway", "kube_deployment", 0, Some(0.5), Some(GIB), "Running"),
        ];
        let full = ContainerUsage { cpu_p95: 0.2, cpu_avg: 0.1, memory_max: 300.0 * MIB, memory_avg: 200.0 * MIB, hours: 24.0 };
        let u = usage(&[("billing", full.clone()), ("orders-db", ContainerUsage { hours: 2.0, ..full.clone() }), ("api-gateway", full)]);
        let hpa = KubeAutoscaler { kind: "HorizontalPodAutoscaler".into(), name: "api-gateway".into(), cluster: "shop-prod".into(), namespace: "checkout".into(), target_kind: "Deployment".into(), target_name: "api-gateway".into(), ..blank_autoscaler() };
        let got = recommendations(&S, &rows, &u, &[hpa]);
        let by = |n: &str| got.iter().find(|w| w.name == n).unwrap();
        assert_eq!(by("billing").not_ready, ["No CPU request on container app"]);
        assert_eq!(by("billing").containers[0].recommended_cpu, Some(0.23), "a recommendation is still computed");
        assert_eq!(by("orders-db").not_ready, ["Less than 6 hours of usage"]);
        assert_eq!(by("api-gateway").autoscaler.as_deref(), Some("HorizontalPodAutoscaler/api-gateway"));
        assert_eq!(by("api-gateway").not_ready, ["Scaled by HorizontalPodAutoscaler api-gateway"]);
    }

    #[test]
    fn a_workload_without_usage_has_no_recommendation_and_no_savings() {
        let rows = vec![workload(KubeKind::Deployments, "catalog-service", 2), pod("catalog-service", "kube_deployment", 0, Some(0.5), Some(GIB), "Running")];
        let got = recommendations(&S, &rows, &WorkloadUsage::new(), &[]);
        assert_eq!((got[0].containers[0].recommended_cpu, got[0].projected_savings), (None, 0.0));
        assert_eq!(got[0].not_ready, ["No usage metrics"]);
    }

    fn blank_autoscaler() -> KubeAutoscaler {
        KubeAutoscaler {
            uid: String::new(),
            cluster: String::new(),
            namespace: String::new(),
            name: String::new(),
            kind: String::new(),
            owner: None,
            profile: None,
            created_at: None,
            target_kind: String::new(),
            target_name: String::new(),
            target_exists: false,
            horizontal: false,
            vertical: false,
            min_replicas: None,
            max_replicas: None,
            current_replicas: None,
            desired_replicas: None,
            pods_running: 0,
            pods_pending: 0,
            pods_failing: 0,
            recommender: None,
            problems: Vec::new(),
        }
    }

    fn dpa(name: &str, target: &str, extra: Value, status: Value) -> CustomObject {
        let mut spec = json!({ "targetRef": { "apiVersion": "apps/v1", "kind": "Deployment", "name": target }, "owner": "Local" });
        spec.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        CustomObject {
            cluster: "shop-prod".into(),
            kind: "DatadogPodAutoscaler".into(),
            uid: format!("dpa-{name}"),
            namespace: "checkout".into(),
            name: name.into(),
            doc: json!({ "spec": spec, "status": status }),
            ..Default::default()
        }
    }

    #[test]
    fn autoscalers_carry_their_dimensions_target_pods_and_problems() {
        let mut rows = vec![workload(KubeKind::Deployments, "storefront-api", 3)];
        rows.extend((0..3).map(|i| pod("storefront-api", "kube_deployment", i, Some(0.5), Some(GIB), if i == 2 { "Pending" } else { "Running" })));
        rows.push(KubeResource {
            uid: "hpa-1".into(),
            kind: Some(KubeKind::HorizontalPodAutoscalers),
            cluster: "shop-prod".into(),
            namespace: Some("checkout".into()),
            name: "legacy-worker".into(),
            target: Some("Deployment/legacy-worker".into()),
            replicas_range: Some("1-4".into()),
            ..Default::default()
        });
        let mut both = dpa(
            "storefront-api",
            "storefront-api",
            json!({ "applyPolicy": { "mode": "Apply", "update": { "strategy": "Auto" } }, "constraints": { "minReplicas": 2, "maxReplicas": 10 } }),
            json!({ "currentReplicas": 3, "horizontal": { "target": { "desiredReplicas": 4 } } }),
        );
        both.annotations.insert(RECOMMENDER_ANNOTATION.into(), r#"{"endpoint": "https://intake.observecraft.example.com/api/v2/autoscaling/recommend?api_key=ocik_x"}"#.into());
        both.owner_kind = "DatadogPodAutoscalerClusterProfile".into();
        both.owner_name = "datadog-optimize-balance".into();
        let vertical_only = dpa(
            "billing",
            "billing",
            json!({ "applyPolicy": { "scaleUp": { "strategy": "Disabled" }, "scaleDown": { "strategy": "Disabled" } } }),
            json!({ "conditions": [{ "type": "Error", "status": "True", "message": "Unable to get target: deployments.apps \"billing\" not found" }] }),
        );
        let got = autoscalers(&rows, &[both, vertical_only]);
        let by = |n: &str| got.iter().find(|a| a.name == n).unwrap();

        let a = by("storefront-api");
        assert_eq!((a.kind.as_str(), a.owner.as_deref(), a.profile.as_deref()), ("DatadogPodAutoscaler", Some("Local"), Some("datadog-optimize-balance")));
        assert_eq!((a.horizontal, a.vertical, a.min_replicas, a.max_replicas, a.current_replicas, a.desired_replicas), (true, true, Some(2), Some(10), Some(3), Some(4)));
        assert_eq!((a.target_exists, a.pods_running, a.pods_pending), (true, 2, 1));
        assert_eq!(a.recommender.as_deref(), Some("https://intake.observecraft.example.com/api/v2/autoscaling/recommend?api_key=ocik_x"));
        assert!(a.problems.is_empty());

        let b = by("billing");
        assert_eq!((b.horizontal, b.vertical, b.target_exists), (false, true, false));
        assert_eq!(b.problems, ["Target Deployment billing not found", "Unable to get target: deployments.apps \"billing\" not found"]);

        let h = by("legacy-worker");
        assert_eq!((h.kind.as_str(), h.horizontal, h.vertical, h.min_replicas, h.max_replicas), ("HorizontalPodAutoscaler", true, false, Some(1), Some(4)));
        assert_eq!(h.problems, ["Target Deployment legacy-worker not found"]);
    }

    fn node(cluster: &str, name: &str, cpu: f64, mem: f64) -> KubeResource {
        KubeResource {
            uid: format!("node-{name}"),
            kind: Some(KubeKind::Nodes),
            cluster: cluster.into(),
            name: name.into(),
            cpu: Some(KubeUtilization { capacity: Some(cpu), ..Default::default() }),
            memory: Some(KubeUtilization { capacity: Some(mem), ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn clusters_report_their_node_autoscaler_idle_capacity_and_savings() {
        let mut rows = vec![node("shop-prod", "i-0a1b2c3d4e5f60001", 4.0, 16.0 * GIB), node("shop-prod", "i-0a1b2c3d4e5f60002", 4.0, 16.0 * GIB), node("shop-prod", "i-0a1b2c3d4e5f60003", 4.0, 16.0 * GIB)];
        let mut p = pod("storefront-api", "kube_deployment", 0, Some(2.0), Some(4.0 * GIB), "Running");
        p.cpu = Some(KubeUtilization { requests: Some(2.0), ..Default::default() });
        p.memory = Some(KubeUtilization { requests: Some(4.0 * GIB), ..Default::default() });
        rows.push(p);
        rows.push(KubeResource { uid: "ca".into(), kind: Some(KubeKind::Deployments), cluster: "shop-staging".into(), namespace: Some("kube-system".into()), name: "cluster-autoscaler".into(), ..Default::default() });
        rows.push(node("shop-staging", "i-0a1b2c3d4e5f60004", 2.0, 8.0 * GIB));
        let w = WorkloadRecommendation {
            uid: "w".into(),
            cluster: "shop-prod".into(),
            namespace: "checkout".into(),
            kind: "Deployment".into(),
            name: "storefront-api".into(),
            service: None,
            replicas: 1,
            containers: vec![],
            cpu_idle: 1.0,
            memory_unused: GIB,
            requests_cost: 100.0,
            usage_cost: 40.0,
            idle_cost: 60.0,
            projected_savings: 25.0,
            not_ready: vec![],
            agent_setup: true,
            autoscaler: None,
            profile: None,
        };
        let pools = vec![CustomObject { cluster: "shop-dev".into(), kind: "NodePool".into(), name: "default".into(), ..Default::default() }];
        let got = cluster_scaling(&S, &rows, &[w], &pools);
        let by = |n: &str| got.iter().find(|c| c.cluster == n).unwrap();
        let prod = by("shop-prod");
        assert_eq!((prod.autoscaling, prod.nodes, prod.cpu_capacity, prod.cpu_requests), (NodeAutoscaler::None, 3, 12.0, 2.0));
        assert!((prod.cluster_idle_cost - S.monthly(10.0, 44.0 * GIB)).abs() < 1e-6);
        // Idle beyond one (the largest) node of headroom.
        assert!((prod.projected_cluster_savings - S.monthly(6.0, 28.0 * GIB)).abs() < 1e-6);
        assert_eq!((prod.workload_idle_cost, prod.usage_cost, prod.projected_workload_savings), (60.0, 40.0, 25.0));
        assert_eq!(by("shop-staging").autoscaling, NodeAutoscaler::ClusterAutoscaler);
        assert_eq!(by("shop-dev").autoscaling, NodeAutoscaler::Karpenter);
    }
    #[test]
    fn profiles_merge_the_clusters_copies_with_the_saved_ones() {
        let profile = |cluster: &str, name: &str, builtin: bool| CustomObject {
            cluster: cluster.into(),
            kind: "DatadogPodAutoscalerClusterProfile".into(),
            name: name.into(),
            labels: if builtin { BTreeMap::from([("autoscaling.datadoghq.com/managed-by".to_string(), "datadog".to_string())]) } else { BTreeMap::new() },
            doc: json!({ "spec": { "template": { "constraints": { "minReplicas": 2 } } } }),
            ..Default::default()
        };
        let objects = vec![profile("shop-prod", "datadog-optimize-cost", true), profile("shop-prod", "checkout-balanced", false), profile("shop-staging", "checkout-balanced", false)];
        let mut labelled = workload(KubeKind::Deployments, "storefront-api", 3);
        labelled.labels.insert(PROFILE_LABEL.into(), "checkout-balanced".into());
        let saved = vec![("p-1".to_string(), "checkout-balanced".to_string(), json!({ "constraints": { "minReplicas": 3 } })), ("p-2".to_string(), "batch-vertical".to_string(), json!({}))];
        let got = profiles(&[labelled], &objects, &saved);
        assert_eq!(got.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["batch-vertical", "checkout-balanced", "datadog-optimize-cost"]);
        let by = |n: &str| got.iter().find(|p| p.name == n).unwrap();
        let custom = by("checkout-balanced");
        assert_eq!((custom.source, custom.id.as_deref(), custom.clusters.clone(), custom.workloads), (ProfileSource::Cluster, Some("p-1"), vec!["shop-prod".to_string(), "shop-staging".to_string()], 1));
        assert_eq!(custom.template, json!({ "constraints": { "minReplicas": 2 } }), "the deployed template is what runs");
        assert_eq!((by("datadog-optimize-cost").source, by("batch-vertical").source), (ProfileSource::Builtin, ProfileSource::Saved));
        assert!(by("batch-vertical").clusters.is_empty());
    }

    #[test]
    fn the_summary_adds_up_clusters_and_workloads() {
        let c = |name: &str, autoscaling: NodeAutoscaler| ClusterScaling {
            cluster: name.into(),
            autoscaling,
            nodes: 1,
            cpu_capacity: 0.0,
            memory_capacity: 0.0,
            cpu_requests: 0.0,
            memory_requests: 0.0,
            cluster_idle_cost: 10.0,
            workload_idle_cost: 20.0,
            usage_cost: 30.0,
            projected_workload_savings: 5.0,
            projected_cluster_savings: 4.0,
        };
        let w = |autoscaler: Option<&str>| WorkloadRecommendation {
            uid: "w".into(),
            cluster: "shop-prod".into(),
            namespace: "checkout".into(),
            kind: "Deployment".into(),
            name: "w".into(),
            service: None,
            replicas: 1,
            containers: vec![],
            cpu_idle: 0.0,
            memory_unused: 0.0,
            requests_cost: 0.0,
            usage_cost: 0.0,
            idle_cost: 0.0,
            projected_savings: 0.0,
            not_ready: vec![],
            agent_setup: true,
            autoscaler: autoscaler.map(str::to_string),
            profile: None,
        };
        let got = summary(&[c("shop-prod", NodeAutoscaler::Karpenter), c("shop-staging", NodeAutoscaler::None)], &[w(None), w(Some("DatadogPodAutoscaler/x")), w(None)]);
        assert_eq!(
            got,
            AutoscalingSummary {
                usage_cost: 60.0,
                workload_idle_cost: 40.0,
                cluster_idle_cost: 20.0,
                workload_savings: 10.0,
                cluster_savings: 8.0,
                workloads: 3,
                unscaled_workloads: 2,
                clusters: 2,
                unscaled_clusters: 1,
            }
        );
    }
    /// Objects and container usage as the Datadog Agent sends them, through the ingest tables: the
    /// workload's recommendation, its DatadogPodAutoscaler and profiles come back for its org only;
    /// settings and saved profiles round-trip.
    #[tokio::test]
    async fn the_overview_reads_objects_usage_settings_and_profiles_of_the_org() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let ctx = crate::test_support::admin_of_new_org(st, "k8s-autoscaling").await;
        let org = ctx.org_id.clone();
        let pod_name = "storefront-api-7d9f8c6b5-abcde";
        let bodies = [
            json!({ "kind": "Node", "metadata": { "uid": format!("{org}-node"), "name": "i-0a1b2c3d4e5f60001" }, "status": { "allocatable": { "cpu": "4", "memory": "16Gi" } } }),
            json!({ "kind": "Deployment", "metadata": { "uid": format!("{org}-dep"), "name": "storefront-api", "namespace": "checkout", "labels": { PROFILE_LABEL: "checkout-balanced" } }, "spec": { "replicas": 2 }, "status": { "replicas": 2, "readyReplicas": 2 } }),
            json!({ "kind": "ReplicaSet", "metadata": { "uid": format!("{org}-rs"), "name": "storefront-api-7d9f8c6b5", "namespace": "checkout", "ownerReferences": [{ "kind": "Deployment", "name": "storefront-api", "uid": format!("{org}-dep"), "controller": true }] }, "spec": { "replicas": 2 } }),
            json!({ "kind": "Pod", "metadata": { "uid": format!("{org}-pod"), "name": pod_name, "namespace": "checkout", "ownerReferences": [{ "kind": "ReplicaSet", "name": "storefront-api-7d9f8c6b5", "uid": format!("{org}-rs"), "controller": true }] },
                    "spec": { "nodeName": "i-0a1b2c3d4e5f60001", "containers": [{ "name": "app", "image": "registry.example.com/storefront-api:2.4.0", "resources": { "requests": { "cpu": "1", "memory": "2Gi" } } }] },
                    "status": { "phase": "Running", "containerStatuses": [{ "name": "app", "ready": true, "restartCount": 0, "state": { "running": {} } }] } }),
            json!({ "kind": "CustomResourceDefinition", "metadata": { "uid": format!("{org}-crd"), "name": super::super::kubernetes_integration::DPA_CRD }, "spec": { "group": "datadoghq.com" } }),
            json!({ "kind": "DatadogPodAutoscaler", "metadata": { "uid": format!("{org}-dpa"), "name": "storefront-api", "namespace": "checkout" },
                    "spec": { "owner": "Local", "targetRef": { "apiVersion": "apps/v1", "kind": "Deployment", "name": "storefront-api" }, "constraints": { "minReplicas": 2, "maxReplicas": 6 } }, "status": { "currentReplicas": 2 } }),
            json!({ "kind": "DatadogPodAutoscalerClusterProfile", "metadata": { "uid": format!("{org}-profile"), "name": "checkout-balanced" }, "spec": { "template": { "constraints": { "minReplicas": 2 } } } }),
        ];
        let ch = st.ch(&org);
        ch.query("INSERT INTO otel.otel_k8s_objects (Timestamp, Body, ResourceAttributes, ScopeName) SELECT now64(9), arrayJoin(?), map('oc.org_id', ?, 'k8s.cluster.name', 'shop-prod', 'oc.signal', 'k8s.objects'), 'datadog.orchestrator'")
            .bind(bodies.iter().map(Value::to_string).collect::<Vec<_>>())
            .bind(&org)
            .execute()
            .await
            .expect("insert objects");
        // 8 hours of usage: 0.2 cores (in nanocores) and 512 MiB.
        for (metric, value) in [("kubernetes.cpu.usage.total", 0.2e9), ("kubernetes.memory.working_set", 512.0 * MIB)] {
            ch.query(
                "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
                 SELECT map('oc.org_id', ?, 'k8s.cluster.name', 'shop-prod', 'k8s.namespace.name', 'checkout', 'k8s.pod.name', ?, 'k8s.container.name', 'app'), ?, map(), now64(9) - toIntervalMinute(arrayJoin([480, 240, 1])), ?",
            )
            .bind(&org)
            .bind(pod_name)
            .bind(metric)
            .bind(value)
            .execute()
            .await
            .expect("insert usage");
        }

        let Json(got) = overview(State(st.clone()), OrgId(org.clone())).await.unwrap();
        let w = got.workloads.iter().find(|w| w.name == "storefront-api").expect("workload");
        assert_eq!((w.replicas, w.agent_setup, w.profile.as_deref(), w.autoscaler.as_deref()), (2, true, Some("checkout-balanced"), Some("DatadogPodAutoscaler/storefront-api")));
        let c = &w.containers[0];
        assert_eq!((c.cpu_request, c.memory_request, c.recommended_cpu, c.recommended_memory), (Some(1.0), Some(2.0 * GIB), Some(0.23), Some(589.0 * MIB)));
        let a = got.autoscalers.iter().find(|a| a.kind == "DatadogPodAutoscaler").expect("dpa");
        assert_eq!((a.target_exists, a.pods_running, a.min_replicas, a.max_replicas), (true, 1, Some(2), Some(6)));
        assert_eq!(got.profiles.iter().map(|p| (p.name.as_str(), p.workloads)).collect::<Vec<_>>(), [("checkout-balanced", 1)]);
        assert_eq!((got.summary.workloads, got.summary.unscaled_workloads, got.clusters[0].nodes), (1, 0, 1));

        let other = crate::test_support::admin_of_new_org(st, "k8s-autoscaling-other").await;
        let Json(none) = overview(State(st.clone()), OrgId(other.org_id.clone())).await.unwrap();
        assert!(none.workloads.is_empty() && none.autoscalers.is_empty() && none.profiles.is_empty());
        assert_eq!(none.settings, AutoscalingSettings::default());

        let prices = AutoscalingSettings { cpu_hour_price: 0.05, memory_gib_hour_price: 0.006 };
        let _ = update_settings(State(st.clone()), Admin(ctx.clone()), Json(prices.clone())).await.unwrap();
        assert!(matches!(update_settings(State(st.clone()), Admin(ctx.clone()), Json(AutoscalingSettings { cpu_hour_price: -1.0, ..prices.clone() })).await, Err(ApiError::BadRequest(_))));
        assert_eq!(load_settings(st, &org).await.unwrap(), prices);
        assert_eq!(load_settings(st, &other.org_id).await.unwrap(), AutoscalingSettings::default());

        let input = |name: &str| Json(ProfileInput { name: name.into(), template: json!({ "constraints": { "minReplicas": 3 } }) });
        let Json(saved) = create_profile(State(st.clone()), Admin(ctx.clone()), input("batch-vertical")).await.unwrap();
        assert!(matches!(create_profile(State(st.clone()), Admin(ctx.clone()), input("batch-vertical")).await, Err(ApiError::Conflict(_))));
        assert!(matches!(create_profile(State(st.clone()), Admin(ctx.clone()), input("Batch Vertical")).await, Err(ApiError::BadRequest(_))));
        let _ = update_profile(State(st.clone()), Admin(ctx.clone()), Path(saved.id.clone()), input("batch-vertical-2")).await.unwrap();
        assert!(matches!(update_profile(State(st.clone()), Admin(other.clone()), Path(saved.id.clone()), input("x")).await, Err(ApiError::NotFound(_))));
        assert!(matches!(delete_profile(State(st.clone()), Admin(other.clone()), Path(saved.id.clone())).await, Err(ApiError::NotFound(_))));
        assert_eq!(saved_profiles(st, &org).await.unwrap().iter().map(|p| p.1.as_str()).collect::<Vec<_>>(), ["batch-vertical-2"]);
        delete_profile(State(st.clone()), Admin(ctx.clone()), Path(saved.id)).await.unwrap();
        assert!(saved_profiles(st, &org).await.unwrap().is_empty());
    }
}
