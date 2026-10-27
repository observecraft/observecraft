//! Kubernetes and ECS Explorers (Datadog's orchestrator explorer), built from what the collectors send:
//! - objects: `otel.k8s_resources` (latest state of each object, refreshed every minute by the
//!   k8sobjects receiver; objects not seen for [`STALE_MINUTES`] are gone),
//! - events: `otel.k8s_events`,
//! - usage: kubeletstats pod / container / node gauges (or the Datadog Agent's kubelet check:
//!   kubernetes.cpu.usage.total, kubernetes.memory.working_set), averaged over the last 15 minutes like
//!   Datadog's resource utilization columns. Requests and limits come from the pod specs.
//!
//! - ECS tasks and containers: the awsecscontainermetrics receiver's task metadata metrics;
//!   containers and container images are derived from pod specs and ECS containers.
//!
//! Like hosts, an org's inventory is small enough to filter, facet, group and sort in memory.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{glob, parse_query, Term};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::metrics::{name_list, now_ms, points_subquery, with_window, OPEN_END_MS};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// Objects are pulled every minute; one not seen for this long has been deleted.
const STALE_MINUTES: u32 = 3;
/// Usage gauges: OpenTelemetry (kubeletstats; the `usage` names are the newer spellings), the
/// Datadog Agent's kubelet check, and ECS.
const K8S_USAGE: &[&str] = &[
    "k8s.pod.cpu.utilization",
    "k8s.pod.cpu.usage",
    "container.cpu.utilization",
    "container.cpu.usage",
    "k8s.node.cpu.utilization",
    "k8s.node.cpu.usage",
    "k8s.pod.memory.working_set",
    "container.memory.working_set",
    "k8s.node.memory.working_set",
];
const DD_USAGE: &[&str] = &["kubernetes.cpu.usage.total", "kubernetes.memory.working_set"];
const ECS_USAGE: &[&str] = &[
    "ecs.task.cpu.reserved",
    "container.cpu.reserved",
    "ecs.task.memory.reserved",
    "container.memory.reserved",
    "ecs.task.cpu.usage.vcpu",
    "container.cpu.usage.vcpu",
    "ecs.task.memory.usage",
    "container.memory.usage",
];
const MAX_OBJECTS: u64 = 50_000;

/// The resource types of the explorer's "Select Resources" menu.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum KubeKind {
    /// Processes of every host (process.* metrics: the Datadog Agent's live processes, or the
    /// OpenTelemetry hostmetrics process scraper).
    Processes,
    Clusters,
    Nodes,
    Namespaces,
    Pods,
    Deployments,
    ReplicaSets,
    DaemonSets,
    StatefulSets,
    Jobs,
    CronJobs,
    Services,
    Ingresses,
    PersistentVolumeClaims,
    PersistentVolumes,
    HorizontalPodAutoscalers,
    NetworkPolicies,
    PodDisruptionBudgets,
    LimitRanges,
    ResourceQuotas,
    ServiceAccounts,
    Roles,
    RoleBindings,
    ClusterRoles,
    ClusterRoleBindings,
    StorageClasses,
    CustomResourceDefinitions,
    /// Every container of every pod (and of ECS tasks).
    Containers,
    #[serde(rename = "container_images")]
    ContainerImages,
    #[serde(rename = "ecs_tasks")]
    EcsTasks,
    #[serde(rename = "ecs_task_definitions")]
    EcsTaskDefinitions,
    #[serde(rename = "ecs_services")]
    EcsServices,
    #[serde(rename = "ecs_clusters")]
    EcsClusters,
}

impl KubeKind {
    const ALL: [KubeKind; 33] = [
        KubeKind::Clusters,
        KubeKind::Nodes,
        KubeKind::Namespaces,
        KubeKind::Pods,
        KubeKind::Deployments,
        KubeKind::ReplicaSets,
        KubeKind::DaemonSets,
        KubeKind::StatefulSets,
        KubeKind::Jobs,
        KubeKind::CronJobs,
        KubeKind::Services,
        KubeKind::Ingresses,
        KubeKind::PersistentVolumeClaims,
        KubeKind::PersistentVolumes,
        KubeKind::HorizontalPodAutoscalers,
        KubeKind::NetworkPolicies,
        KubeKind::PodDisruptionBudgets,
        KubeKind::LimitRanges,
        KubeKind::ResourceQuotas,
        KubeKind::ServiceAccounts,
        KubeKind::Roles,
        KubeKind::RoleBindings,
        KubeKind::ClusterRoles,
        KubeKind::ClusterRoleBindings,
        KubeKind::StorageClasses,
        KubeKind::CustomResourceDefinitions,
        KubeKind::Containers,
        KubeKind::ContainerImages,
        KubeKind::EcsTasks,
        KubeKind::EcsTaskDefinitions,
        KubeKind::EcsServices,
        KubeKind::EcsClusters,
        KubeKind::Processes,
    ];

    /// The Kubernetes `kind` (None for resources derived from other data).
    fn api_kind(self) -> Option<&'static str> {
        Some(match self {
            KubeKind::Clusters
            | KubeKind::Containers
            | KubeKind::ContainerImages
            | KubeKind::EcsTasks
            | KubeKind::EcsTaskDefinitions
            | KubeKind::EcsServices
            | KubeKind::EcsClusters
            | KubeKind::Processes => return None,
            KubeKind::Nodes => "Node",
            KubeKind::Namespaces => "Namespace",
            KubeKind::Pods => "Pod",
            KubeKind::Deployments => "Deployment",
            KubeKind::ReplicaSets => "ReplicaSet",
            KubeKind::DaemonSets => "DaemonSet",
            KubeKind::StatefulSets => "StatefulSet",
            KubeKind::Jobs => "Job",
            KubeKind::CronJobs => "CronJob",
            KubeKind::Services => "Service",
            KubeKind::Ingresses => "Ingress",
            KubeKind::PersistentVolumeClaims => "PersistentVolumeClaim",
            KubeKind::PersistentVolumes => "PersistentVolume",
            KubeKind::HorizontalPodAutoscalers => "HorizontalPodAutoscaler",
            KubeKind::NetworkPolicies => "NetworkPolicy",
            KubeKind::PodDisruptionBudgets => "PodDisruptionBudget",
            KubeKind::LimitRanges => "LimitRange",
            KubeKind::ResourceQuotas => "ResourceQuota",
            KubeKind::ServiceAccounts => "ServiceAccount",
            KubeKind::Roles => "Role",
            KubeKind::RoleBindings => "RoleBinding",
            KubeKind::ClusterRoles => "ClusterRole",
            KubeKind::ClusterRoleBindings => "ClusterRoleBinding",
            KubeKind::StorageClasses => "StorageClass",
            KubeKind::CustomResourceDefinitions => "CustomResourceDefinition",
        })
    }

    fn from_api(kind: &str) -> Option<KubeKind> {
        KubeKind::ALL.into_iter().find(|k| k.api_kind() == Some(kind))
    }

    /// Facets shown on the left of the explorer, after cluster / namespace / status.
    fn facets(self) -> &'static [&'static str] {
        match self {
            KubeKind::Pods => &["kube_node", "kube_ownerref_kind", "kube_deployment", "kube_daemon_set", "kube_stateful_set", "kube_job", "kube_qos", "service", "env", "short_image"],
            KubeKind::Nodes => &["kube_node_role", "kubelet_version", "os"],
            KubeKind::Deployments | KubeKind::DaemonSets | KubeKind::StatefulSets | KubeKind::ReplicaSets => &["service", "env", "short_image"],
            KubeKind::Jobs => &["kube_cronjob"],
            KubeKind::Services => &["kube_service_type"],
            KubeKind::PersistentVolumeClaims | KubeKind::PersistentVolumes => &["kube_storage_class"],
            KubeKind::Containers => &["kube_node", "kube_deployment", "pod_name", "short_image", "ecs_cluster", "task_family"],
            KubeKind::ContainerImages => &["short_image", "image_tag"],
            KubeKind::EcsTasks => &["task_health", "task_launch_type", "ecs_cluster", "task_family", "ecs_service", "region"],
            KubeKind::EcsTaskDefinitions => &["task_launch_type", "region"],
            KubeKind::EcsServices => &["task_launch_type", "ecs_cluster", "region"],
            KubeKind::EcsClusters => &["region"],
            KubeKind::Processes => &[
                "command",
                "user",
                "host",
                "kube_namespace",
                "pod_name",
                "container_id",
            ],
            _ => &[],
        }
    }

    fn namespaced(self) -> bool {
        !matches!(
            self,
            KubeKind::Clusters
                | KubeKind::Nodes
                | KubeKind::Namespaces
                | KubeKind::PersistentVolumes
                | KubeKind::ClusterRoles
                | KubeKind::ClusterRoleBindings
                | KubeKind::StorageClasses
                | KubeKind::CustomResourceDefinitions
        ) && !self.is_ecs()
            && self != KubeKind::ContainerImages
            && self != KubeKind::Processes
    }

    fn is_ecs(self) -> bool {
        matches!(self, KubeKind::EcsTasks | KubeKind::EcsTaskDefinitions | KubeKind::EcsServices | KubeKind::EcsClusters)
    }
}

/// Status colour, as in Datadog's status pills.
#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum KubeHealth {
    Ok,
    Warning,
    Error,
    #[default]
    Info,
}

/// CPU in cores or memory in bytes. `usage` is the 15-minute average.
#[derive(Serialize, ToSchema, Clone, Default, Debug)]
pub struct KubeUtilization {
    pub usage: Option<f64>,
    pub requests: Option<f64>,
    pub limits: Option<f64>,
    /// Allocatable capacity (nodes and clusters).
    pub capacity: Option<f64>,
}

impl KubeUtilization {
    fn add(&mut self, o: &KubeUtilization) {
        let sum = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        };
        self.usage = sum(self.usage, o.usage);
        self.requests = sum(self.requests, o.requests);
        self.limits = sum(self.limits, o.limits);
        self.capacity = sum(self.capacity, o.capacity);
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct KubeObjectRef {
    pub kind: String,
    pub name: String,
    pub uid: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KubeContainer {
    pub name: String,
    pub image: String,
    pub ready: bool,
    pub restarts: u32,
    /// running / waiting / terminated
    pub state: String,
    /// CrashLoopBackOff, OOMKilled, Completed, …
    pub reason: Option<String>,
    pub started_at: Option<i64>,
    pub init: bool,
    pub cpu: KubeUtilization,
    pub memory: KubeUtilization,
}

/// One row of the explorer. Kind-specific columns are null for other kinds.
#[derive(Serialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KubeResource {
    pub uid: String,
    pub kind: Option<KubeKind>,
    pub cluster: String,
    pub namespace: Option<String>,
    pub name: String,
    /// Epoch ms.
    pub created_at: Option<i64>,
    pub last_seen: i64,
    pub status: String,
    pub health: KubeHealth,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    /// Datadog tags (`kube_namespace:shop`, `kube_deployment:checkout`, …) followed by labels.
    pub tags: Vec<String>,
    pub owner: Option<KubeObjectRef>,
    pub cpu: Option<KubeUtilization>,
    pub memory: Option<KubeUtilization>,
    /// "1/2" containers (pods), replicas (workloads) or nodes (daemonsets).
    pub ready: Option<String>,
    pub restarts: Option<u32>,
    pub node: Option<String>,
    pub ip: Option<String>,
    pub qos: Option<String>,
    pub desired: Option<u32>,
    pub current: Option<u32>,
    pub up_to_date: Option<u32>,
    pub available: Option<u32>,
    pub pods: Option<u32>,
    pub nodes: Option<u32>,
    pub images: Vec<String>,
    pub containers: Vec<KubeContainer>,
    /// Services: type, cluster IP, external address, "80/TCP,443/TCP".
    pub service_type: Option<String>,
    pub cluster_ip: Option<String>,
    pub external: Option<String>,
    pub ports: Option<String>,
    pub selector: Option<String>,
    /// CronJobs.
    pub schedule: Option<String>,
    pub suspend: Option<bool>,
    pub last_schedule: Option<i64>,
    pub active: Option<u32>,
    /// Jobs: "1/1" succeeded / completions, and run time in seconds.
    pub completions: Option<String>,
    pub duration_sec: Option<f64>,
    /// Nodes and clusters.
    pub version: Option<String>,
    pub roles: Option<String>,
    pub os_image: Option<String>,
    pub runtime: Option<String>,
    pub instance_type: Option<String>,
    pub zone: Option<String>,
    /// Storage.
    pub storage_class: Option<String>,
    pub capacity: Option<String>,
    pub access_modes: Option<String>,
    pub volume: Option<String>,
    /// HPAs and ingresses: scale target or backend, "min-max", hosts.
    pub target: Option<String>,
    pub replicas_range: Option<String>,
    pub hosts: Option<String>,
    pub ingress_class: Option<String>,
    /// ECS: `family:revision`, `fargate` / `ec2`, HEALTHY / UNHEALTHY / UNKNOWN, reservations
    /// (vCPU, bytes), and task / service counts.
    pub task_definition: Option<String>,
    pub launch_type: Option<String>,
    pub task_health: Option<String>,
    pub cpu_reservation: Option<f64>,
    pub memory_reservation: Option<f64>,
    pub tasks: Option<u32>,
    pub services: Option<u32>,
    /// Container images: tag and containers running it.
    pub image_tag: Option<String>,
    pub running_containers: Option<u32>,
    /// Processes: owner and the host's CPU count (CPU is then shown normalized to the host).
    pub user: Option<String>,
    pub host_cpus: Option<u32>,
}

impl KubeResource {
    /// Every value of a tag key (labels and containers make some keys multi-valued).
    fn values<'a>(&'a self, key: &str) -> Vec<&'a str> {
        match key {
            "name" => vec![self.name.as_str()],
            "status" => vec![self.status.as_str()],
            "kube_cluster_name" | "cluster" => vec![self.cluster.as_str()],
            _ => {
                let prefix = format!("{key}:");
                self.tags.iter().filter_map(|t| t.strip_prefix(&prefix)).collect()
            }
        }
    }

    fn matches(&self, terms: &[Term]) -> bool {
        terms.iter().all(|t| match t {
            Term::Text { text, negate } => self.name.to_lowercase().contains(&text.to_lowercase()) != *negate,
            Term::Tag { key, values, negate } => self.values(key).iter().any(|v| values.iter().any(|p| glob(p, v))) != *negate,
        })
    }
}

// ================================================================ loading

/// Raw objects plus usage, loaded once per request.
struct Inventory {
    objects: Vec<(KubeKind, String, i64, Value)>,
    /// pod uid → (cpu cores, memory bytes)
    pod_usage: HashMap<String, (f64, f64)>,
    /// (pod uid, container) → (cpu cores, memory bytes)
    container_usage: HashMap<(String, String), (f64, f64)>,
    /// (cluster, node) → (cpu cores, memory bytes)
    node_usage: HashMap<(String, String), (f64, f64)>,
    /// ECS tasks (container empty) and their containers, from the awsecscontainermetrics receiver.
    ecs: Vec<EcsRow>,
    /// Latest state of every process (last 5 minutes).
    processes: Vec<ProcRow>,
    /// host → CPU count (Datadog Agent host metadata).
    host_cpus: HashMap<String, u32>,
}

#[derive(clickhouse::Row, Deserialize)]
struct ProcRow {
    host: String,
    pid: String,
    exe: String,
    cmdline: String,
    user: String,
    created: String,
    container: String,
    container_name: String,
    pod: String,
    namespace: String,
    cluster: String,
    cpu: f64,
    rss: f64,
    seen: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct EcsRow {
    arn: String,
    container: String,
    res_json: String,
    seen: i64,
    cpu_reserved: Option<f64>,
    mem_reserved: Option<f64>,
    cpu: Option<f64>,
    mem: Option<f64>,
}

async fn load(st: &AppState, org: &str) -> Result<Inventory, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Obj {
        cluster: String,
        kind: String,
        seen: i64,
        body: String,
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct Usage {
        cluster: String,
        node: String,
        pod: String,
        container: String,
        cpu: f64,
        mem: f64,
    }
    let objects = st
        .ch(org)
        .query(&format!(
            "SELECT Cluster AS cluster, Kind AS kind, toUnixTimestamp64Milli(LastSeen) AS seen, Body AS body FROM otel.k8s_resources FINAL \
             WHERE OrgId = {{oc_org:String}} AND LastSeen > now64(3) - INTERVAL {STALE_MINUTES} MINUTE LIMIT {MAX_OBJECTS}"
        ))
        .fetch_all::<Obj>();
    // Points of the last 15 minutes (5 for processes), from the new storage once it has them.
    let now = now_ms();
    let (recent, procs_from, end) = (now - 15 * 60_000, now - 5 * 60_000, now + OPEN_END_MS);
    let usage_src = points_subquery("gauge", K8S_USAGE);
    let dd_src = points_subquery("gauge", DD_USAGE);
    let ecs_src = points_subquery("gauge", ECS_USAGE);
    let proc_cpu_src = points_subquery("gauge", &["process.cpu.utilization"]);
    let proc_mem_src = points_subquery("sum", &["process.memory.usage"]);
    // Pod, container and node gauges in one pass; the `usage` names are the newer spellings.
    let usage = with_window(st.ch(org), recent, end)
        .query(&format!(
            "SELECT ResourceAttributes['k8s.cluster.name'] AS cluster, ResourceAttributes['k8s.node.name'] AS node, ResourceAttributes['k8s.pod.uid'] AS pod, \
             ResourceAttributes['k8s.container.name'] AS container, \
             ifNotFinite(avgIf(Value, MetricName IN ('k8s.pod.cpu.utilization', 'k8s.pod.cpu.usage', 'container.cpu.utilization', 'container.cpu.usage', 'k8s.node.cpu.utilization', 'k8s.node.cpu.usage')), -1) AS cpu, \
             ifNotFinite(avgIf(Value, MetricName IN ('k8s.pod.memory.working_set', 'container.memory.working_set', 'k8s.node.memory.working_set')), -1) AS mem \
             FROM {usage_src} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 15 MINUTE AND MetricName IN ({names}) \
             GROUP BY cluster, node, pod, container",
            names = name_list(K8S_USAGE),
        ))
        .fetch_all::<Usage>();
    // The Datadog Agent's kubelet check: per-container nanocores and bytes, tagged with the pod's
    // namespace and name (no uid); node-level rows have no pod.
    #[derive(clickhouse::Row, Deserialize)]
    struct DdUsage {
        cluster: String,
        node: String,
        namespace: String,
        pod: String,
        container: String,
        cpu: f64,
        mem: f64,
    }
    let dd_usage = with_window(st.ch(org), recent, end)
        .query(&format!(
            "SELECT ResourceAttributes['k8s.cluster.name'] AS cluster, ResourceAttributes['host.name'] AS node, \
             ResourceAttributes['k8s.namespace.name'] AS namespace, ResourceAttributes['k8s.pod.name'] AS pod, ResourceAttributes['k8s.container.name'] AS container, \
             ifNotFinite(avgIf(Value / 1e9, MetricName = 'kubernetes.cpu.usage.total'), -1) AS cpu, \
             ifNotFinite(avgIf(Value, MetricName = 'kubernetes.memory.working_set'), -1) AS mem \
             FROM {dd_src} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 15 MINUTE \
             AND MetricName IN ({names}) \
             GROUP BY cluster, node, namespace, pod, container",
            names = name_list(DD_USAGE),
        ))
        .fetch_all::<DdUsage>();
    // ECS task and container gauges (vCPU, MiB reservations; vCPU and bytes used).
    let ecs = with_window(st.ch(org), recent, end)
        .query(&format!(
            "SELECT ResourceAttributes['aws.ecs.task.arn'] AS arn, ResourceAttributes['container.name'] AS container, toJSONString(any(ResourceAttributes)) AS res_json, \
             toUnixTimestamp64Milli(max(TimeUnix)) AS seen, \
             maxIfOrNull(Value, MetricName IN ('ecs.task.cpu.reserved', 'container.cpu.reserved')) AS cpu_reserved, \
             maxIfOrNull(Value, MetricName IN ('ecs.task.memory.reserved', 'container.memory.reserved')) AS mem_reserved, \
             avgIfOrNull(Value, MetricName IN ('ecs.task.cpu.usage.vcpu', 'container.cpu.usage.vcpu')) AS cpu, \
             avgIfOrNull(Value, MetricName IN ('ecs.task.memory.usage', 'container.memory.usage')) AS mem \
             FROM {ecs_src} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 15 MINUTE \
             AND MetricName IN ({names}) \
             AND ResourceAttributes['aws.ecs.task.arn'] != '' GROUP BY arn, container LIMIT 50000",
            names = name_list(ECS_USAGE),
        ))
        .fetch_all::<EcsRow>();
    // Processes: the latest CPU (fraction of a core) and RSS of each (host, pid).
    let processes = with_window(st.ch(org), procs_from, end)
        .query(&format!(
            "SELECT ResourceAttributes['host.name'] AS host, ResourceAttributes['process.pid'] AS pid, \
             any(ResourceAttributes['process.executable.name']) AS exe, any(ResourceAttributes['process.command_line']) AS cmdline, \
             any(ResourceAttributes['process.owner']) AS user, any(ResourceAttributes['process.create_time']) AS created, \
             any(ResourceAttributes['container.id']) AS container, any(ResourceAttributes['k8s.container.name']) AS container_name, any(ResourceAttributes['k8s.pod.name']) AS pod, \
             any(ResourceAttributes['k8s.namespace.name']) AS namespace, any(ResourceAttributes['k8s.cluster.name']) AS cluster, \
             ifNotFinite(argMaxIf(Value, TimeUnix, MetricName = 'process.cpu.utilization'), 0) AS cpu, \
             ifNotFinite(argMaxIf(Value, TimeUnix, MetricName = 'process.memory.usage'), 0) AS rss, toInt64(toUnixTimestamp64Milli(max(TimeUnix))) AS seen \
             FROM ( \
               SELECT ResourceAttributes, MetricName, Value, TimeUnix FROM {proc_cpu_src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.cpu.utilization' AND TimeUnix > now() - INTERVAL 5 MINUTE \
               UNION ALL SELECT ResourceAttributes, MetricName, Value, TimeUnix FROM {proc_mem_src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.memory.usage' AND TimeUnix > now() - INTERVAL 5 MINUTE) \
             WHERE pid != '' GROUP BY host, pid LIMIT 50000"
        ))
        .fetch_all::<ProcRow>();
    let host_cpus = sqlx::query_as::<_, (String, i32)>(
        "SELECT host, cpu_cores FROM datadog_hosts WHERE org_id = $1 AND cpu_cores IS NOT NULL",
    )
    .bind(org)
    .fetch_all(&st.postgres);
    let (objects, usage, dd_usage, ecs, processes, host_cpus) = tokio::try_join!(
        async { objects.await.map_err(ApiError::from) },
        async { usage.await.map_err(ApiError::from) },
        async { dd_usage.await.map_err(ApiError::from) },
        async { ecs.await.map_err(ApiError::from) },
        async { processes.await.map_err(ApiError::from) },
        async { host_cpus.await.map_err(ApiError::from) }
    )?;
    let host_cpus = host_cpus
        .into_iter()
        .filter(|(_, c)| *c > 0)
        .map(|(h, c)| (h, c as u32))
        .collect();
    let mut inv = Inventory {
        objects: Vec::with_capacity(objects.len()),
        pod_usage: HashMap::new(),
        container_usage: HashMap::new(),
        node_usage: HashMap::new(),
        ecs,
        processes,
        host_cpus,
    };
    for o in objects {
        let (Some(kind), Ok(body)) = (KubeKind::from_api(&o.kind), serde_json::from_str::<Value>(&o.body)) else { continue };
        inv.objects.push((kind, o.cluster, o.seen, body));
    }
    for u in usage {
        let v = (u.cpu, u.mem);
        if !u.container.is_empty() {
            inv.container_usage.insert((u.pod, u.container), v);
        } else if !u.pod.is_empty() {
            inv.pod_usage.insert(u.pod, v);
        } else if !u.node.is_empty() {
            inv.node_usage.insert((u.cluster, u.node), v);
        }
    }
    if !dd_usage.is_empty() {
        let pods = PodIndex::new(inv.objects.iter().filter(|(k, ..)| *k == KubeKind::Pods).filter_map(|(_, cluster, _, b)| {
            let m = &b["metadata"];
            Some((cluster.as_str(), m["namespace"].as_str()?, m["name"].as_str()?, m["uid"].as_str()?))
        }));
        // Node name → cluster, to find the node a Datadog host is.
        let node_clusters: HashMap<&str, &str> =
            inv.objects.iter().filter(|(k, ..)| *k == KubeKind::Nodes).filter_map(|(_, cluster, _, b)| Some((b["metadata"]["name"].as_str()?, cluster.as_str()))).collect();
        let mut container_usage = Vec::new();
        let mut node_usage = Vec::new();
        let mut container_on_node = Vec::new();
        for u in dd_usage {
            if u.pod.is_empty() {
                if let Some((cluster, node)) = node_of_host(&u.node, &u.cluster, &node_clusters) {
                    node_usage.push(((cluster.to_string(), node.to_string()), (u.cpu, u.mem)));
                }
            } else {
                // The kubelet check reports containers only: a node's usage is the sum of its containers'.
                if let Some((cluster, node)) = node_of_host(&u.node, &u.cluster, &node_clusters) {
                    container_on_node.push(((cluster.to_string(), node.to_string()), (u.cpu, u.mem)));
                }
                if let Some(uid) = pods.uid(&u.cluster, &u.namespace, &u.pod) {
                    if !u.container.is_empty() {
                        container_usage.push(((uid.to_string(), u.container), (u.cpu, u.mem)));
                    }
                }
            }
        }
        node_usage.extend(node_totals(container_on_node));
        for (k, v) in container_usage {
            inv.container_usage.entry(k).or_insert(v);
        }
        for (k, v) in node_usage {
            inv.node_usage.entry(k).or_insert(v);
        }
    }
    Ok(inv)
}

/// Pod uids by (cluster, namespace, name), and by (namespace, name) for metrics without a cluster
/// (an Agent with no cluster name, or before its host tags arrive) when only one cluster has the pod.
struct PodIndex<'a> {
    full: HashMap<(&'a str, &'a str, &'a str), &'a str>,
    /// None: the same namespace/name in several clusters.
    by_name: HashMap<(&'a str, &'a str), Option<&'a str>>,
}

impl<'a> PodIndex<'a> {
    fn new(pods: impl Iterator<Item = (&'a str, &'a str, &'a str, &'a str)>) -> Self {
        let mut index = PodIndex { full: HashMap::new(), by_name: HashMap::new() };
        for (cluster, ns, name, uid) in pods {
            index.full.insert((cluster, ns, name), uid);
            index.by_name.entry((ns, name)).and_modify(|u| *u = None).or_insert(Some(uid));
        }
        index
    }

    fn uid(&self, cluster: &str, ns: &str, name: &str) -> Option<&'a str> {
        if cluster.is_empty() {
            self.by_name.get(&(ns, name)).copied().flatten()
        } else {
            self.full.get(&(cluster, ns, name)).copied()
        }
    }
}

/// The Kubernetes node (and its cluster) a Datadog host is: the Agent names it `<node>-<cluster>`.
fn node_of_host<'a>(host: &str, cluster: &str, nodes: &HashMap<&'a str, &'a str>) -> Option<(&'a str, &'a str)> {
    if let Some((node, c)) = nodes.get_key_value(host) {
        return Some((c, node));
    }
    nodes.iter().find(|(node, c)| (cluster.is_empty() || cluster == **c) && host.strip_suffix(**c).and_then(|h| h.strip_suffix('-')) == Some(**node)).map(|(node, c)| (*c, *node))
}

/// Per-node CPU and memory as the sum of its containers' (-1: not measured).
fn node_totals<K: std::hash::Hash + Eq>(rows: impl IntoIterator<Item = (K, (f64, f64))>) -> HashMap<K, (f64, f64)> {
    let add = |total: f64, v: f64| if v < 0.0 { total } else if total < 0.0 { v } else { total + v };
    let mut out: HashMap<K, (f64, f64)> = HashMap::new();
    for (k, (cpu, mem)) in rows {
        let t = out.entry(k).or_insert((-1.0, -1.0));
        *t = (add(t.0, cpu), add(t.1, mem));
    }
    out
}

fn measured(v: f64) -> Option<f64> {
    (v >= 0.0 && v.is_finite()).then_some(v)
}

// ================================================================ parsing objects

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn n(v: &Value, path: &[&str]) -> Option<u32> {
    path.iter().try_fold(v, |v, k| v.get(k)).and_then(Value::as_u64).map(|x| x as u32)
}

fn arr<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    path.iter().try_fold(v, |v, k| v.get(k)).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn map(v: &Value, path: &[&str]) -> BTreeMap<String, String> {
    path.iter()
        .try_fold(v, |v, k| v.get(k))
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default()
}

/// Kubernetes timestamps (`2026-10-25T18:46:38Z`, optionally with fractions) → epoch ms.
fn ts(v: &Value, path: &[&str]) -> Option<i64> {
    parse_time(s(v, path)?)
}

fn parse_time(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400) + hh * 3600 + mm * 60 + ss) * 1000)
}

/// `250m` / `0.5` / `2` cores, `64Mi` / `1G` / `129e6` bytes.
pub fn parse_quantity(q: &str) -> Option<f64> {
    let q = q.trim();
    let split = q.find(|c: char| c.is_ascii_alphabetic() && c != 'e' && c != 'E').unwrap_or(q.len());
    let (num, suffix) = q.split_at(split);
    let v: f64 = num.parse().ok()?;
    let mult = match suffix {
        "" => 1.0,
        "m" => 1e-3,
        "u" => 1e-6,
        "n" => 1e-9,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "Ki" => 1024.0,
        "Mi" => 1024.0_f64.powi(2),
        "Gi" => 1024.0_f64.powi(3),
        "Ti" => 1024.0_f64.powi(4),
        "Pi" => 1024.0_f64.powi(5),
        _ => return None,
    };
    Some(v * mult)
}

fn quantity(v: &Value, path: &[&str]) -> Option<f64> {
    parse_quantity(s(v, path)?)
}

/// `docker.io/library/busybox:1.36` → (`docker.io/library/busybox`, `busybox`, `1.36`).
fn image_parts(image: &str) -> (String, String, String) {
    let no_digest = image.split('@').next().unwrap_or(image);
    let (name, tag) = match no_digest.rsplit_once(':') {
        Some((n, t)) if !t.contains('/') => (n, t),
        _ => (no_digest, "latest"),
    };
    let short = name.rsplit('/').next().unwrap_or(name);
    (name.to_string(), short.to_string(), tag.to_string())
}

fn controller(meta: &Value) -> Option<KubeObjectRef> {
    let refs = arr(meta, &["ownerReferences"]);
    let r = refs.iter().find(|r| r.get("controller").and_then(Value::as_bool) == Some(true)).or(refs.first())?;
    Some(KubeObjectRef { kind: s(r, &["kind"])?.to_string(), name: s(r, &["name"])?.to_string(), uid: s(r, &["uid"]).unwrap_or_default().to_string() })
}

/// Pod status as `kubectl get pods` prints it, with its colour.
fn pod_status(body: &Value) -> (String, KubeHealth) {
    if s(body, &["metadata", "deletionTimestamp"]).is_some() {
        return ("Terminating".into(), KubeHealth::Warning);
    }
    let phase = s(body, &["status", "phase"]).unwrap_or("Unknown");
    let statuses = arr(body, &["status", "containerStatuses"]);
    for c in arr(body, &["status", "initContainerStatuses"]).iter().chain(statuses) {
        if let Some(r) = s(c, &["state", "waiting", "reason"]) {
            let bad = matches!(r, "CrashLoopBackOff" | "ImagePullBackOff" | "ErrImagePull" | "InvalidImageName" | "CreateContainerConfigError" | "CreateContainerError" | "RunContainerError");
            return (r.to_string(), if bad { KubeHealth::Error } else { KubeHealth::Warning });
        }
    }
    if phase != "Succeeded" {
        for c in statuses {
            if let Some(r) = s(c, &["state", "terminated", "reason"]) {
                return (r.to_string(), if r == "Completed" { KubeHealth::Info } else { KubeHealth::Error });
            }
        }
    }
    let all_ready = !statuses.is_empty() && statuses.iter().all(|c| c.get("ready").and_then(Value::as_bool) == Some(true));
    let health = match phase {
        "Running" if all_ready => KubeHealth::Ok,
        "Running" | "Pending" => KubeHealth::Warning,
        "Succeeded" => KubeHealth::Info,
        "Failed" | "Unknown" => KubeHealth::Error,
        _ => KubeHealth::Info,
    };
    (if phase == "Succeeded" { "Completed".into() } else { phase.to_string() }, health)
}

fn ready_health(ready: u32, desired: u32) -> (String, KubeHealth) {
    if desired == 0 {
        ("Scaled Down".into(), KubeHealth::Info)
    } else if ready >= desired {
        ("Available".into(), KubeHealth::Ok)
    } else if ready == 0 {
        ("Unavailable".into(), KubeHealth::Error)
    } else {
        ("Progressing".into(), KubeHealth::Warning)
    }
}

fn condition<'a>(body: &'a Value, kind: &str) -> Option<&'a Value> {
    arr(body, &["status", "conditions"]).iter().find(|c| s(c, &["type"]) == Some(kind))
}

fn container_infos(body: &Value, pod_uid: &str, inv: &Inventory) -> Vec<KubeContainer> {
    let mut out = Vec::new();
    for (spec_key, status_key, init) in [("initContainers", "initContainerStatuses", true), ("containers", "containerStatuses", false)] {
        let statuses = arr(body, &["status", status_key]);
        for c in arr(body, &["spec", spec_key]) {
            let name = s(c, &["name"]).unwrap_or_default().to_string();
            let st = statuses.iter().find(|x| s(x, &["name"]) == Some(name.as_str()));
            let state = st.and_then(|x| x.get("state")).and_then(Value::as_object).and_then(|m| m.keys().next().cloned()).unwrap_or_else(|| "waiting".into());
            let reason = st.and_then(|x| s(x, &["state", &state, "reason"])).map(str::to_string);
            let usage = inv.container_usage.get(&(pod_uid.to_string(), name.clone()));
            out.push(KubeContainer {
                image: s(c, &["image"]).unwrap_or_default().to_string(),
                ready: st.and_then(|x| x.get("ready")).and_then(Value::as_bool).unwrap_or(false),
                restarts: st.and_then(|x| n(x, &["restartCount"])).unwrap_or(0),
                started_at: st.and_then(|x| ts(x, &["state", "running", "startedAt"])),
                reason,
                state,
                init,
                cpu: KubeUtilization {
                    usage: usage.and_then(|u| measured(u.0)),
                    requests: quantity(c, &["resources", "requests", "cpu"]),
                    limits: quantity(c, &["resources", "limits", "cpu"]),
                    capacity: None,
                },
                memory: KubeUtilization {
                    usage: usage.and_then(|u| measured(u.1)),
                    requests: quantity(c, &["resources", "requests", "memory"]),
                    limits: quantity(c, &["resources", "limits", "memory"]),
                    capacity: None,
                },
                name,
            });
        }
    }
    out
}

/// Sum of a pod's regular containers; a limit only counts when every container sets one.
fn pod_utilization(containers: &[KubeContainer], pick: impl Fn(&KubeContainer) -> &KubeUtilization, usage: Option<f64>) -> KubeUtilization {
    let regular: Vec<&KubeUtilization> = containers.iter().filter(|c| !c.init).map(pick).collect();
    let sum = |f: &dyn Fn(&KubeUtilization) -> Option<f64>| {
        let vals: Vec<f64> = regular.iter().filter_map(|u| f(u)).collect();
        (!vals.is_empty()).then(|| vals.iter().sum())
    };
    let limits = if regular.iter().all(|u| u.limits.is_some()) { sum(&|u| u.limits) } else { None };
    KubeUtilization { usage: usage.or_else(|| sum(&|u| u.usage)), requests: sum(&|u| u.requests), limits, capacity: None }
}

/// Turns one object into a row (without the cross-object rollups, done in [`build`]).
/// A pod's `pod_phase` and `kube_condition_ready` tags (as the Datadog Agent tags pods), for queries.
fn pod_tags(status: &Value) -> Vec<String> {
    let phase = s(status, &["phase"]).map(|p| format!("pod_phase:{}", p.to_lowercase()));
    let ready = arr(status, &["conditions"])
        .iter()
        .find(|c| s(c, &["type"]) == Some("Ready"))
        .and_then(|c| s(c, &["status"]))
        .map(|v| format!("kube_condition_ready:{}", v.to_lowercase()));
    phase.into_iter().chain(ready).collect()
}

fn to_row(kind: KubeKind, cluster: &str, seen: i64, body: &Value, inv: &Inventory) -> KubeResource {
    let meta = body.get("metadata").cloned().unwrap_or(Value::Null);
    let uid = s(&meta, &["uid"]).unwrap_or_default().to_string();
    let name = s(&meta, &["name"]).unwrap_or_default().to_string();
    let namespace = s(&meta, &["namespace"]).map(str::to_string);
    let mut annotations = map(&meta, &["annotations"]);
    annotations.remove("kubectl.kubernetes.io/last-applied-configuration");
    let mut r = KubeResource {
        kind: Some(kind),
        cluster: cluster.to_string(),
        created_at: ts(&meta, &["creationTimestamp"]),
        last_seen: seen,
        labels: map(&meta, &["labels"]),
        annotations,
        owner: controller(&meta),
        namespace,
        name,
        uid,
        ..Default::default()
    };
    let spec = body.get("spec").cloned().unwrap_or(Value::Null);
    let status = body.get("status").cloned().unwrap_or(Value::Null);
    let pod_template_images = |path: &[&str]| -> Vec<String> { arr(&spec, path).iter().filter_map(|c| s(c, &["image"]).map(str::to_string)).collect() };
    match kind {
        KubeKind::Pods => {
            (r.status, r.health) = pod_status(body);
            r.containers = container_infos(body, &r.uid, inv);
            let regular: Vec<&KubeContainer> = r.containers.iter().filter(|c| !c.init).collect();
            r.ready = Some(format!("{}/{}", regular.iter().filter(|c| c.ready).count(), regular.len()));
            r.restarts = Some(r.containers.iter().map(|c| c.restarts).sum());
            r.node = s(&spec, &["nodeName"]).map(str::to_string);
            r.ip = s(&status, &["podIP"]).map(str::to_string);
            r.qos = s(&status, &["qosClass"]).map(str::to_string);
            r.tags = pod_tags(&status);
            r.images = regular.iter().map(|c| c.image.clone()).collect();
            let usage = inv.pod_usage.get(&r.uid);
            r.cpu = Some(pod_utilization(&r.containers, |c| &c.cpu, usage.and_then(|u| measured(u.0))));
            r.memory = Some(pod_utilization(&r.containers, |c| &c.memory, usage.and_then(|u| measured(u.1))));
        }
        KubeKind::Deployments | KubeKind::StatefulSets | KubeKind::ReplicaSets => {
            let desired = n(&spec, &["replicas"]).unwrap_or(1);
            let ready = n(&status, &["readyReplicas"]).unwrap_or(0);
            r.desired = Some(desired);
            r.current = Some(n(&status, &["replicas"]).unwrap_or(0));
            r.up_to_date = n(&status, &["updatedReplicas"]).or(Some(0)).filter(|_| kind != KubeKind::ReplicaSets);
            r.available = n(&status, &["availableReplicas"]).or(Some(0)).filter(|_| kind != KubeKind::StatefulSets);
            r.ready = Some(format!("{ready}/{desired}"));
            (r.status, r.health) = ready_health(ready, desired);
            r.images = pod_template_images(&["template", "spec", "containers"]);
            r.selector = selector(&spec);
        }
        KubeKind::DaemonSets => {
            let desired = n(&status, &["desiredNumberScheduled"]).unwrap_or(0);
            let ready = n(&status, &["numberReady"]).unwrap_or(0);
            r.desired = Some(desired);
            r.current = Some(n(&status, &["currentNumberScheduled"]).unwrap_or(0));
            r.up_to_date = Some(n(&status, &["updatedNumberScheduled"]).unwrap_or(0));
            r.available = Some(n(&status, &["numberAvailable"]).unwrap_or(0));
            r.ready = Some(format!("{ready}/{desired}"));
            (r.status, r.health) = ready_health(ready, desired);
            r.images = pod_template_images(&["template", "spec", "containers"]);
            r.selector = selector(&spec);
        }
        KubeKind::Jobs => {
            let succeeded = n(&status, &["succeeded"]).unwrap_or(0);
            r.completions = Some(format!("{succeeded}/{}", n(&spec, &["completions"]).unwrap_or(1)));
            r.active = Some(n(&status, &["active"]).unwrap_or(0));
            let start = ts(&status, &["startTime"]);
            let end = ts(&status, &["completionTime"]).unwrap_or(seen);
            r.duration_sec = start.map(|s| (end - s).max(0) as f64 / 1000.0);
            let done = |t: &str| condition(body, t).and_then(|c| s(c, &["status"])) == Some("True");
            (r.status, r.health) = if done("Complete") {
                ("Complete".into(), KubeHealth::Ok)
            } else if done("Failed") {
                ("Failed".into(), KubeHealth::Error)
            } else if spec.get("suspend").and_then(Value::as_bool) == Some(true) {
                ("Suspended".into(), KubeHealth::Info)
            } else {
                ("Running".into(), KubeHealth::Warning)
            };
            r.images = pod_template_images(&["template", "spec", "containers"]);
        }
        KubeKind::CronJobs => {
            r.schedule = s(&spec, &["schedule"]).map(str::to_string);
            r.suspend = Some(spec.get("suspend").and_then(Value::as_bool).unwrap_or(false));
            r.active = Some(arr(&status, &["active"]).len() as u32);
            r.last_schedule = ts(&status, &["lastScheduleTime"]);
            (r.status, r.health) = if r.suspend == Some(true) { ("Suspended".into(), KubeHealth::Info) } else { ("Scheduled".into(), KubeHealth::Ok) };
            r.images = pod_template_images(&["jobTemplate", "spec", "template", "spec", "containers"]);
        }
        KubeKind::Services => {
            r.service_type = s(&spec, &["type"]).map(str::to_string);
            r.cluster_ip = s(&spec, &["clusterIP"]).map(str::to_string);
            let ingress: Vec<String> = arr(&status, &["loadBalancer", "ingress"]).iter().filter_map(|i| s(i, &["hostname"]).or(s(i, &["ip"])).map(str::to_string)).collect();
            let external: Vec<String> = arr(&spec, &["externalIPs"]).iter().filter_map(|v| v.as_str().map(str::to_string)).chain(ingress).collect();
            r.external = (!external.is_empty()).then(|| external.join(","));
            let ports: Vec<String> = arr(&spec, &["ports"])
                .iter()
                .map(|p| {
                    let port = n(p, &["port"]).unwrap_or(0);
                    let proto = s(p, &["protocol"]).unwrap_or("TCP");
                    match n(p, &["nodePort"]) {
                        Some(np) => format!("{port}:{np}/{proto}"),
                        None => format!("{port}/{proto}"),
                    }
                })
                .collect();
            r.ports = Some(ports.join(","));
            r.selector = spec.get("selector").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default())).collect::<Vec<_>>().join(","));
            r.status = "Active".into();
            r.health = KubeHealth::Ok;
        }
        KubeKind::Ingresses => {
            r.ingress_class = s(&spec, &["ingressClassName"]).map(str::to_string);
            let hosts: Vec<String> = arr(&spec, &["rules"]).iter().filter_map(|x| s(x, &["host"]).map(str::to_string)).collect();
            r.hosts = Some(if hosts.is_empty() { "*".into() } else { hosts.join(",") });
            let addr: Vec<String> = arr(&status, &["loadBalancer", "ingress"]).iter().filter_map(|i| s(i, &["hostname"]).or(s(i, &["ip"])).map(str::to_string)).collect();
            r.external = (!addr.is_empty()).then(|| addr.join(","));
            (r.status, r.health) = if addr.is_empty() { ("Pending".into(), KubeHealth::Warning) } else { ("Active".into(), KubeHealth::Ok) };
        }
        KubeKind::Nodes => {
            let ready = condition(body, "Ready").and_then(|c| s(c, &["status"])) == Some("True");
            let unschedulable = spec.get("unschedulable").and_then(Value::as_bool) == Some(true);
            (r.status, r.health) = match (ready, unschedulable) {
                (true, false) => ("Ready".into(), KubeHealth::Ok),
                (true, true) => ("Ready,SchedulingDisabled".into(), KubeHealth::Warning),
                (false, _) => ("NotReady".into(), KubeHealth::Error),
            };
            let roles: Vec<&str> = r.labels.keys().filter_map(|k| k.strip_prefix("node-role.kubernetes.io/")).collect();
            r.roles = Some(if roles.is_empty() { "<none>".into() } else { roles.join(",") });
            r.version = s(&status, &["nodeInfo", "kubeletVersion"]).map(str::to_string);
            r.os_image = s(&status, &["nodeInfo", "osImage"]).map(str::to_string);
            r.runtime = s(&status, &["nodeInfo", "containerRuntimeVersion"]).map(str::to_string);
            r.instance_type = r.labels.get("node.kubernetes.io/instance-type").cloned();
            r.zone = r.labels.get("topology.kubernetes.io/zone").cloned();
            r.ip = arr(&status, &["addresses"]).iter().find(|a| s(a, &["type"]) == Some("InternalIP")).and_then(|a| s(a, &["address"])).map(str::to_string);
            let usage = inv.node_usage.get(&(cluster.to_string(), r.name.clone()));
            r.cpu = Some(KubeUtilization { usage: usage.and_then(|u| measured(u.0)), capacity: quantity(&status, &["allocatable", "cpu"]), ..Default::default() });
            r.memory = Some(KubeUtilization { usage: usage.and_then(|u| measured(u.1)), capacity: quantity(&status, &["allocatable", "memory"]), ..Default::default() });
            r.capacity = s(&status, &["allocatable", "pods"]).map(|p| format!("{p} pods"));
        }
        KubeKind::Namespaces => {
            let phase = s(&status, &["phase"]).unwrap_or("Active");
            r.health = if phase == "Active" { KubeHealth::Ok } else { KubeHealth::Warning };
            r.status = phase.to_string();
        }
        KubeKind::PersistentVolumeClaims | KubeKind::PersistentVolumes => {
            let phase = s(&status, &["phase"]).unwrap_or("Pending");
            r.health = match phase {
                "Bound" => KubeHealth::Ok,
                "Available" | "Released" => KubeHealth::Info,
                "Pending" => KubeHealth::Warning,
                _ => KubeHealth::Error,
            };
            r.status = phase.to_string();
            r.storage_class = s(&spec, &["storageClassName"]).map(str::to_string);
            r.capacity = s(&status, &["capacity", "storage"]).or(s(&spec, &["capacity", "storage"])).or(s(&spec, &["resources", "requests", "storage"])).map(str::to_string);
            r.access_modes = Some(arr(&spec, &["accessModes"]).iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","));
            r.volume = if kind == KubeKind::PersistentVolumes {
                s(&spec, &["claimRef", "name"]).map(|c| format!("{}/{c}", s(&spec, &["claimRef", "namespace"]).unwrap_or_default()))
            } else {
                s(&spec, &["volumeName"]).map(str::to_string)
            };
        }
        KubeKind::HorizontalPodAutoscalers => {
            r.target = s(&spec, &["scaleTargetRef", "name"]).map(|t| format!("{}/{t}", s(&spec, &["scaleTargetRef", "kind"]).unwrap_or_default()));
            let max = n(&spec, &["maxReplicas"]).unwrap_or(0);
            r.replicas_range = Some(format!("{}-{max}", n(&spec, &["minReplicas"]).unwrap_or(1)));
            r.current = Some(n(&status, &["currentReplicas"]).unwrap_or(0));
            r.desired = Some(n(&status, &["desiredReplicas"]).unwrap_or(0));
            let able = condition(body, "AbleToScale").and_then(|c| s(c, &["status"])) != Some("False");
            (r.status, r.health) = if !able {
                ("Unable to Scale".into(), KubeHealth::Error)
            } else if r.current >= Some(max) && max > 0 {
                ("At Max Replicas".into(), KubeHealth::Warning)
            } else {
                ("Active".into(), KubeHealth::Ok)
            };
        }
        KubeKind::RoleBindings | KubeKind::ClusterRoleBindings => {
            r.target = s(body, &["roleRef", "name"]).map(|n| format!("{}/{n}", s(body, &["roleRef", "kind"]).unwrap_or_default()));
            let subjects: Vec<String> = arr(body, &["subjects"]).iter().filter_map(|x| s(x, &["name"]).map(|n| format!("{}/{n}", s(x, &["kind"]).unwrap_or_default()))).collect();
            r.hosts = Some(subjects.join(","));
            (r.status, r.health) = ("Active".into(), KubeHealth::Ok);
        }
        KubeKind::Roles | KubeKind::ClusterRoles => {
            r.desired = Some(arr(body, &["rules"]).len() as u32);
            (r.status, r.health) = ("Active".into(), KubeHealth::Ok);
        }
        KubeKind::StorageClasses => {
            r.target = s(body, &["provisioner"]).map(str::to_string);
            r.volume = s(body, &["reclaimPolicy"]).map(str::to_string);
            (r.status, r.health) = ("Active".into(), KubeHealth::Ok);
        }
        KubeKind::CustomResourceDefinitions => {
            r.target = s(&spec, &["group"]).map(str::to_string);
            r.volume = s(&spec, &["scope"]).map(str::to_string);
            r.version = arr(&spec, &["versions"]).iter().find(|v| v.get("storage").and_then(Value::as_bool) == Some(true)).and_then(|v| s(v, &["name"])).map(str::to_string);
            (r.status, r.health) = ("Established".into(), KubeHealth::Ok);
        }
        KubeKind::PodDisruptionBudgets => {
            r.desired = n(&status, &["desiredHealthy"]);
            r.current = n(&status, &["currentHealthy"]);
            r.available = n(&status, &["disruptionsAllowed"]);
            (r.status, r.health) = if r.current >= r.desired { ("Healthy".into(), KubeHealth::Ok) } else { ("Unhealthy".into(), KubeHealth::Warning) };
        }
        KubeKind::NetworkPolicies | KubeKind::LimitRanges | KubeKind::ResourceQuotas | KubeKind::ServiceAccounts => {
            (r.status, r.health) = ("Active".into(), KubeHealth::Ok);
        }
        // Derived in `build`, not from objects.
        _ => {}
    }
    r
}

fn selector(spec: &Value) -> Option<String> {
    let labels = map(spec, &["selector", "matchLabels"]);
    (!labels.is_empty()).then(|| labels.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","))
}

/// The tag key Datadog uses for a workload kind's name.
fn workload_tag(api_kind: &str) -> Option<&'static str> {
    Some(match api_kind {
        "Deployment" => "kube_deployment",
        "ReplicaSet" => "kube_replica_set",
        "DaemonSet" => "kube_daemon_set",
        "StatefulSet" => "kube_stateful_set",
        "Job" => "kube_job",
        "CronJob" => "kube_cronjob",
        _ => return None,
    })
}

/// Every row of the org, with ownership, rollups and tags resolved.
fn build(inv: &Inventory) -> Vec<KubeResource> {
    let mut rows: Vec<KubeResource> = inv.objects.iter().map(|(k, c, seen, b)| to_row(*k, c, *seen, b, inv)).collect();
    let by_uid: HashMap<String, usize> = rows.iter().enumerate().map(|(i, r)| (r.uid.clone(), i)).collect();

    // Owner chain of each object (pod → replicaset → deployment, job → cronjob).
    let ancestors = |i: usize| -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = i;
        while let Some(o) = rows[cur].owner.as_ref().and_then(|o| by_uid.get(&o.uid)) {
            if out.contains(o) || out.len() > 4 {
                break;
            }
            out.push(*o);
            cur = *o;
        }
        out
    };
    let chains: Vec<Vec<usize>> = (0..rows.len()).map(ancestors).collect();

    // Workload, namespace, node and cluster rollups of pod usage.
    let mut cpu: HashMap<usize, KubeUtilization> = HashMap::new();
    let mut mem: HashMap<usize, KubeUtilization> = HashMap::new();
    let mut pods: HashMap<usize, u32> = HashMap::new();
    let ns_idx: HashMap<(String, String), usize> = rows.iter().enumerate().filter(|(_, r)| r.kind == Some(KubeKind::Namespaces)).map(|(i, r)| ((r.cluster.clone(), r.name.clone()), i)).collect();
    let node_idx: HashMap<(String, String), usize> = rows.iter().enumerate().filter(|(_, r)| r.kind == Some(KubeKind::Nodes)).map(|(i, r)| ((r.cluster.clone(), r.name.clone()), i)).collect();
    for (i, r) in rows.iter().enumerate().filter(|(_, r)| r.kind == Some(KubeKind::Pods)) {
        let running = !matches!(r.status.as_str(), "Completed" | "Failed");
        let mut targets = chains[i].clone();
        if let Some(ns) = r.namespace.as_ref().and_then(|ns| ns_idx.get(&(r.cluster.clone(), ns.clone()))) {
            targets.push(*ns);
        }
        let on_node = r.node.as_ref().and_then(|n| node_idx.get(&(r.cluster.clone(), n.clone()))).copied();
        // Static pods are owned by their node: count them there once.
        targets.retain(|t| Some(*t) != on_node);
        for t in targets.iter().chain(on_node.iter()) {
            *pods.entry(*t).or_default() += 1;
            if !running {
                continue;
            }
            let (c, m) = (r.cpu.clone().unwrap_or_default(), r.memory.clone().unwrap_or_default());
            // A node's usage comes from its own metrics; pods add their requests and limits.
            let (c, m) = if Some(*t) == on_node { (KubeUtilization { usage: None, ..c }, KubeUtilization { usage: None, ..m }) } else { (c, m) };
            cpu.entry(*t).or_default().add(&c);
            mem.entry(*t).or_default().add(&m);
        }
    }
    for (i, r) in rows.iter_mut().enumerate() {
        if let Some(p) = pods.get(&i) {
            r.pods = Some(*p);
        }
        let kind = r.kind.unwrap_or(KubeKind::Pods);
        if matches!(kind, KubeKind::Deployments | KubeKind::ReplicaSets | KubeKind::DaemonSets | KubeKind::StatefulSets | KubeKind::Jobs | KubeKind::CronJobs | KubeKind::Namespaces) {
            r.cpu = Some(cpu.remove(&i).unwrap_or_default());
            r.memory = Some(mem.remove(&i).unwrap_or_default());
        } else if kind == KubeKind::Nodes {
            let (c, m) = (cpu.remove(&i).unwrap_or_default(), mem.remove(&i).unwrap_or_default());
            if let Some(x) = r.cpu.as_mut() {
                (x.requests, x.limits) = (c.requests, c.limits);
            }
            if let Some(x) = r.memory.as_mut() {
                (x.requests, x.limits) = (m.requests, m.limits);
            }
            r.pods.get_or_insert(0);
        }
    }

    // Tags: Datadog's names, the owners' names, unified service tags, then labels.
    let snapshot: Vec<(Option<KubeKind>, String, Option<KubeObjectRef>)> = rows.iter().map(|r| (r.kind, r.name.clone(), r.owner.clone())).collect();
    for (i, r) in rows.iter_mut().enumerate() {
        let mut tags = vec![format!("kube_cluster_name:{}", r.cluster)];
        if let Some(ns) = &r.namespace {
            tags.push(format!("kube_namespace:{ns}"));
        }
        let kind = r.kind.unwrap_or(KubeKind::Pods);
        let own = match kind {
            KubeKind::Pods => Some("pod_name"),
            KubeKind::Nodes => Some("kube_node"),
            KubeKind::Services => Some("kube_service"),
            KubeKind::Namespaces => Some("kube_namespace"),
            KubeKind::Ingresses => Some("kube_ingress"),
            KubeKind::PersistentVolumeClaims => Some("persistentvolumeclaim"),
            KubeKind::PersistentVolumes => Some("persistentvolume"),
            KubeKind::HorizontalPodAutoscalers => Some("horizontalpodautoscaler"),
            k => k.api_kind().and_then(workload_tag),
        };
        if let Some(t) = own {
            if kind != KubeKind::Namespaces {
                tags.push(format!("{t}:{}", r.name));
            }
        }
        for a in &chains[i] {
            if let (Some(k), name, _) = &snapshot[*a] {
                if let Some(t) = k.api_kind().and_then(workload_tag) {
                    tags.push(format!("{t}:{name}"));
                }
            }
        }
        if let Some(o) = &r.owner {
            tags.push(format!("kube_ownerref_kind:{}", o.kind.to_lowercase()));
            tags.push(format!("kube_ownerref_name:{}", o.name));
        }
        if let Some(n) = &r.node {
            tags.push(format!("kube_node:{n}"));
        }
        if let Some(q) = &r.qos {
            tags.push(format!("kube_qos:{q}"));
        }
        // Set by `to_row` (e.g. `pod_phase`).
        tags.append(&mut r.tags);
        if let Some(t) = &r.service_type {
            tags.push(format!("kube_service_type:{t}"));
        }
        if let Some(c) = &r.storage_class {
            tags.push(format!("kube_storage_class:{c}"));
        }
        if let Some(v) = &r.version {
            tags.push(format!("kubelet_version:{v}"));
        }
        if let Some(roles) = &r.roles {
            tags.extend(roles.split(',').map(|x| format!("kube_node_role:{x}")));
        }
        if kind == KubeKind::Nodes {
            if let Some(os) = r.labels.get("kubernetes.io/os") {
                tags.push(format!("os:{os}"));
            }
        }
        for c in r.containers.iter().filter(|c| !c.init) {
            tags.push(format!("kube_container_name:{}", c.name));
        }
        let mut seen_img = HashSet::new();
        for img in &r.images {
            let (name, short, tag) = image_parts(img);
            if seen_img.insert(name.clone()) {
                tags.push(format!("image_name:{name}"));
                tags.push(format!("short_image:{short}"));
                tags.push(format!("image_tag:{tag}"));
            }
        }
        for (label, key) in [("tags.datadoghq.com/service", "service"), ("tags.datadoghq.com/env", "env"), ("tags.datadoghq.com/version", "version")] {
            if let Some(v) = r.labels.get(label) {
                tags.push(format!("{key}:{v}"));
            }
        }
        if !tags.iter().any(|t| t.starts_with("service:")) {
            if let Some(v) = r.labels.get("app.kubernetes.io/name").or(r.labels.get("app")) {
                tags.push(format!("service:{v}"));
            }
        }
        tags.extend(r.labels.iter().map(|(k, v)| format!("{k}:{v}")));
        r.tags = tags;
    }

    rows.extend(clusters(&rows));
    let containers = containers(&rows);
    rows.extend(images(&containers));
    rows.extend(containers);
    rows.extend(ecs(inv));
    rows.extend(processes(inv));
    rows
}

/// One row per container of each pod, tagged like its pod.
fn containers(rows: &[KubeResource]) -> Vec<KubeResource> {
    let mut out = Vec::new();
    for p in rows.iter().filter(|r| r.kind == Some(KubeKind::Pods)) {
        for c in p.containers.iter().filter(|c| !c.init) {
            let (status, health) = match (c.state.as_str(), c.reason.as_deref()) {
                ("running", _) => ("Running".to_string(), if c.ready { KubeHealth::Ok } else { KubeHealth::Warning }),
                (_, Some(r)) if r.contains("BackOff") || r.contains("Err") || r == "OOMKilled" || r == "Error" => (r.to_string(), KubeHealth::Error),
                ("terminated", r) => (r.unwrap_or("Exited").to_string(), KubeHealth::Info),
                (_, r) => (r.unwrap_or("Waiting").to_string(), KubeHealth::Warning),
            };
            let (image_name, short, tag) = image_parts(&c.image);
            let mut tags: Vec<String> = p.tags.iter().filter(|t| !t.starts_with("kube_container_name:") && !t.starts_with("image_") && !t.starts_with("short_image:")).cloned().collect();
            tags.extend([format!("kube_container_name:{}", c.name), format!("container_name:{}", c.name), format!("image_name:{image_name}"), format!("short_image:{short}"), format!("image_tag:{tag}")]);
            out.push(KubeResource {
                uid: format!("{}/{}", p.uid, c.name),
                kind: Some(KubeKind::Containers),
                cluster: p.cluster.clone(),
                namespace: p.namespace.clone(),
                name: c.name.clone(),
                created_at: c.started_at,
                last_seen: p.last_seen,
                status,
                health,
                tags,
                owner: Some(KubeObjectRef { kind: "Pod".into(), name: p.name.clone(), uid: p.uid.clone() }),
                cpu: Some(c.cpu.clone()),
                memory: Some(c.memory.clone()),
                restarts: Some(c.restarts),
                node: p.node.clone(),
                images: vec![c.image.clone()],
                ..Default::default()
            });
        }
    }
    out
}

/// Images in use, with the containers running each.
fn images(containers: &[KubeResource]) -> Vec<KubeResource> {
    let mut by: BTreeMap<String, KubeResource> = BTreeMap::new();
    for c in containers {
        let Some(img) = c.images.first() else { continue };
        let (name, short, tag) = image_parts(img);
        let r = by.entry(format!("{name}:{tag}")).or_insert_with(|| KubeResource {
            uid: format!("image:{name}:{tag}"),
            kind: Some(KubeKind::ContainerImages),
            name: name.clone(),
            image_tag: Some(tag.clone()),
            status: "In Use".into(),
            health: KubeHealth::Ok,
            running_containers: Some(0),
            tags: vec![format!("image_name:{name}"), format!("short_image:{short}"), format!("image_tag:{tag}")],
            ..Default::default()
        });
        if c.status == "Running" {
            *r.running_containers.get_or_insert(0) += 1;
        }
        r.last_seen = r.last_seen.max(c.last_seen);
        if !c.cluster.is_empty() && !r.tags.contains(&format!("kube_cluster_name:{}", c.cluster)) {
            r.tags.push(format!("kube_cluster_name:{}", c.cluster));
            r.cluster = if r.cluster.is_empty() { c.cluster.clone() } else { format!("{},{}", r.cluster, c.cluster) };
        }
    }
    by.into_values().collect()
}

/// Processes of every host, as the Processes explorer lists them.
fn processes(inv: &Inventory) -> Vec<KubeResource> {
    inv.processes
        .iter()
        .map(|p| {
            let name = if p.cmdline.is_empty() {
                p.exe.clone()
            } else {
                p.cmdline.clone()
            };
            let mut tags = vec![format!("host:{}", p.host)];
            for (k, v) in [
                ("command", &p.exe),
                ("user", &p.user),
                ("container_id", &p.container),
                ("kube_container_name", &p.container_name),
                ("pod_name", &p.pod),
                ("kube_namespace", &p.namespace),
                ("kube_cluster_name", &p.cluster),
            ] {
                if !v.is_empty() {
                    tags.push(format!("{k}:{v}"));
                }
            }
            let cpus = inv.host_cpus.get(&p.host).copied();
            KubeResource {
                uid: format!("process:{}:{}:{}", p.host, p.pid, p.created),
                kind: Some(KubeKind::Processes),
                cluster: p.cluster.clone(),
                namespace: Some(p.namespace.clone()).filter(|n| !n.is_empty()),
                name,
                created_at: p.created.parse().ok().filter(|t: &i64| *t > 0),
                last_seen: p.seen,
                status: "Running".into(),
                health: KubeHealth::Ok,
                node: Some(p.host.clone()),
                user: Some(p.user.clone()).filter(|u| !u.is_empty()),
                host_cpus: cpus,
                cpu: Some(KubeUtilization {
                    usage: Some(p.cpu.max(0.0)),
                    capacity: cpus.map(f64::from),
                    ..Default::default()
                }),
                memory: Some(KubeUtilization {
                    usage: Some(p.rss.max(0.0)),
                    ..Default::default()
                }),
                owner: Some(p.pod.clone())
                    .filter(|x| !x.is_empty())
                    .map(|pod| KubeObjectRef {
                        kind: "Pod".into(),
                        name: pod,
                        uid: String::new(),
                    }),
                ip: Some(p.pid.clone()),
                tags,
                ..Default::default()
            }
        })
        .collect()
}

/// ECS tasks, their containers, task definitions, services and clusters, from task metadata metrics.
fn ecs(inv: &Inventory) -> Vec<KubeResource> {
    const MIB: f64 = 1024.0 * 1024.0;
    let mut out = Vec::new();
    let mut tasks: Vec<KubeResource> = Vec::new();
    for e in &inv.ecs {
        let res: HashMap<String, String> = serde_json::from_str(&e.res_json).unwrap_or_default();
        let get = |k: &str| res.get(k).filter(|v| !v.is_empty()).cloned();
        let cluster = get("aws.ecs.cluster.name").unwrap_or_default();
        let family = get("aws.ecs.task.family").unwrap_or_default();
        let revision = get("aws.ecs.task.revision").unwrap_or_default();
        let launch = get("aws.ecs.launch_type").map(|l| l.to_lowercase());
        let mut tags = vec![format!("ecs_cluster:{cluster}"), format!("task_family:{family}"), format!("task_version:{revision}"), format!("task_arn:{}", e.arn)];
        for (k, t) in [("aws.ecs.service.name", "ecs_service"), ("cloud.region", "region"), ("cloud.account.id", "aws_account"), ("cloud.availability_zone", "availability_zone")] {
            if let Some(v) = get(k) {
                tags.push(format!("{t}:{v}"));
            }
        }
        if let Some(l) = &launch {
            tags.push(format!("task_launch_type:{l}"));
        }
        let task_id = get("aws.ecs.task.id").unwrap_or_else(|| e.arn.rsplit('/').next().unwrap_or(&e.arn).to_string());
        let usage = |v: Option<f64>| v.filter(|x| x.is_finite() && *x >= 0.0);
        if e.container.is_empty() {
            let status = get("aws.ecs.task.known_status").unwrap_or_else(|| "UNKNOWN".into()).to_uppercase();
            let health = match status.as_str() {
                "RUNNING" => KubeHealth::Ok,
                "STOPPED" | "DEPROVISIONING" | "STOPPING" => KubeHealth::Error,
                _ => KubeHealth::Warning,
            };
            tags.push(format!("task_status:{}", status.to_lowercase()));
            // The task metadata endpoint has no health check result: Datadog shows UNKNOWN too.
            tags.push("task_health:unknown".into());
            tasks.push(KubeResource {
                uid: e.arn.clone(),
                kind: Some(KubeKind::EcsTasks),
                cluster: cluster.clone(),
                name: task_id,
                created_at: get("aws.ecs.task.pull_started_at").and_then(|t| parse_time(&t)),
                last_seen: e.seen,
                status,
                health,
                task_definition: Some(format!("{family}:{revision}")),
                launch_type: launch,
                task_health: Some("UNKNOWN".into()),
                cpu_reservation: usage(e.cpu_reserved),
                memory_reservation: usage(e.mem_reserved).map(|m| m * MIB),
                cpu: Some(KubeUtilization { usage: usage(e.cpu), limits: usage(e.cpu_reserved), ..Default::default() }),
                memory: Some(KubeUtilization { usage: usage(e.mem), limits: usage(e.mem_reserved).map(|m| m * MIB), ..Default::default() }),
                tags,
                ..Default::default()
            });
        } else {
            let status = get("aws.ecs.container.know_status").or(get("aws.ecs.container.known_status")).unwrap_or_else(|| "UNKNOWN".into());
            let image = get("container.image.name").map(|n| format!("{n}:{}", get("container.image.tag").unwrap_or_else(|| "latest".into()))).or(get("container.image.tag")).unwrap_or_default();
            tags.extend([format!("container_name:{}", e.container), format!("task_id:{task_id}")]);
            if !image.is_empty() {
                let (name, short, tag) = image_parts(&image);
                tags.extend([format!("image_name:{name}"), format!("short_image:{short}"), format!("image_tag:{tag}")]);
            }
            let running = status.eq_ignore_ascii_case("RUNNING");
            out.push(KubeResource {
                uid: format!("{}/{}", e.arn, e.container),
                kind: Some(KubeKind::Containers),
                cluster: cluster.clone(),
                name: e.container.clone(),
                created_at: get("aws.ecs.container.started_at").and_then(|t| parse_time(&t)),
                last_seen: e.seen,
                status: if running { "Running".into() } else { status.clone() },
                health: if running { KubeHealth::Ok } else { KubeHealth::Info },
                owner: Some(KubeObjectRef { kind: "Task".into(), name: task_id, uid: e.arn.clone() }),
                cpu: Some(KubeUtilization { usage: usage(e.cpu), limits: usage(e.cpu_reserved), ..Default::default() }),
                memory: Some(KubeUtilization { usage: usage(e.mem), limits: usage(e.mem_reserved).map(|m| m * MIB), ..Default::default() }),
                images: if image.is_empty() { vec![] } else { vec![image] },
                tags,
                ..Default::default()
            });
        }
    }
    // Task definitions, services and clusters roll up their tasks.
    let mut defs: BTreeMap<String, KubeResource> = BTreeMap::new();
    let mut services: BTreeMap<(String, String), KubeResource> = BTreeMap::new();
    let mut clusters: BTreeMap<String, KubeResource> = BTreeMap::new();
    let tag = |t: &KubeResource, k: &str| t.values(k).first().map(|v| v.to_string());
    for t in &tasks {
        let shared: Vec<String> = t.tags.iter().filter(|x| x.starts_with("region:") || x.starts_with("aws_account:") || x.starts_with("task_launch_type:")).cloned().collect();
        let def = t.task_definition.clone().unwrap_or_default();
        let d = defs.entry(def.clone()).or_insert_with(|| KubeResource {
            uid: format!("taskdef:{def}"),
            kind: Some(KubeKind::EcsTaskDefinitions),
            name: def.clone(),
            status: "ACTIVE".into(),
            health: KubeHealth::Ok,
            launch_type: t.launch_type.clone(),
            cpu_reservation: t.cpu_reservation,
            memory_reservation: t.memory_reservation,
            tasks: Some(0),
            tags: [vec![format!("task_family:{}", tag(t, "task_family").unwrap_or_default()), format!("task_version:{}", tag(t, "task_version").unwrap_or_default())], shared.clone()].concat(),
            ..Default::default()
        });
        *d.tasks.get_or_insert(0) += 1;
        d.last_seen = d.last_seen.max(t.last_seen);
        if let Some(svc) = tag(t, "ecs_service") {
            let s = services.entry((t.cluster.clone(), svc.clone())).or_insert_with(|| KubeResource {
                uid: format!("ecs-service:{}/{svc}", t.cluster),
                kind: Some(KubeKind::EcsServices),
                cluster: t.cluster.clone(),
                name: svc.clone(),
                status: "ACTIVE".into(),
                health: KubeHealth::Ok,
                launch_type: t.launch_type.clone(),
                task_definition: t.task_definition.clone(),
                tasks: Some(0),
                tags: [vec![format!("ecs_cluster:{}", t.cluster), format!("ecs_service:{svc}")], shared.clone()].concat(),
                ..Default::default()
            });
            if t.status == "RUNNING" {
                *s.tasks.get_or_insert(0) += 1;
            }
            s.last_seen = s.last_seen.max(t.last_seen);
        }
        let c = clusters.entry(t.cluster.clone()).or_insert_with(|| KubeResource {
            uid: format!("ecs-cluster:{}", t.cluster),
            kind: Some(KubeKind::EcsClusters),
            cluster: t.cluster.clone(),
            name: t.cluster.clone(),
            status: "ACTIVE".into(),
            health: KubeHealth::Ok,
            tasks: Some(0),
            services: Some(0),
            tags: [vec![format!("ecs_cluster:{}", t.cluster)], shared.into_iter().filter(|x| !x.starts_with("task_launch_type:")).collect()].concat(),
            ..Default::default()
        });
        if t.status == "RUNNING" {
            *c.tasks.get_or_insert(0) += 1;
        }
        c.last_seen = c.last_seen.max(t.last_seen);
    }
    for (cluster, _) in services.keys() {
        if let Some(c) = clusters.get_mut(cluster) {
            *c.services.get_or_insert(0) += 1;
        }
    }
    out.extend(tasks);
    out.extend(defs.into_values());
    out.extend(services.into_values());
    out.extend(clusters.into_values());
    out
}

/// Clusters are the sum of their nodes; their version is the newest kubelet's.
fn clusters(rows: &[KubeResource]) -> Vec<KubeResource> {
    let mut out: BTreeMap<String, KubeResource> = BTreeMap::new();
    for r in rows {
        let c = out.entry(r.cluster.clone()).or_insert_with(|| KubeResource {
            uid: format!("cluster:{}", r.cluster),
            kind: Some(KubeKind::Clusters),
            cluster: r.cluster.clone(),
            name: r.cluster.clone(),
            tags: vec![format!("kube_cluster_name:{}", r.cluster)],
            cpu: Some(KubeUtilization::default()),
            memory: Some(KubeUtilization::default()),
            nodes: Some(0),
            pods: Some(0),
            ..Default::default()
        });
        c.last_seen = c.last_seen.max(r.last_seen);
        match r.kind {
            Some(KubeKind::Nodes) => {
                *c.nodes.get_or_insert(0) += 1;
                c.cpu.get_or_insert_default().add(r.cpu.as_ref().unwrap_or(&KubeUtilization::default()));
                c.memory.get_or_insert_default().add(r.memory.as_ref().unwrap_or(&KubeUtilization::default()));
                c.version = c.version.clone().max(r.version.clone());
                c.created_at = match (c.created_at, r.created_at) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                if r.health != KubeHealth::Ok {
                    c.health = KubeHealth::Warning;
                }
            }
            Some(KubeKind::Pods) => *c.pods.get_or_insert(0) += 1,
            _ => {}
        }
    }
    out.into_values()
        .map(|mut c| {
            let nodes = c.nodes.unwrap_or(0);
            if c.health != KubeHealth::Warning {
                c.health = if nodes > 0 { KubeHealth::Ok } else { KubeHealth::Info };
            }
            c.status = if c.health == KubeHealth::Ok { "Healthy".into() } else if nodes == 0 { "No Nodes".into() } else { "Degraded".into() };
            c
        })
        .collect()
}

// ================================================================ explorer

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubeSearchRequest {
    pub kind: KubeKind,
    /// `kube_namespace:shop status:crashloopbackoff app:checkout` + free text on the name.
    #[serde(default)]
    pub query: String,
    /// Tag keys, comma-separated (`kube_namespace`, `kube_cluster_name,kube_namespace,kube_deployment`, a label key, …).
    pub group_by: Option<String>,
    /// Column to sort by: name, status, age, restarts, cpu, memory, namespace, cluster, node, ….
    pub sort: Option<String>,
    #[serde(default)]
    pub desc: bool,
    #[serde(default)]
    pub offset: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
pub struct KubeFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
pub struct KubeFacet {
    pub key: String,
    pub values: Vec<KubeFacetValue>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubeGroup {
    /// The group's values joined with ", " (one per group-by key).
    pub key: String,
    /// The group's value for each group-by key, in order.
    pub values: Vec<String>,
    pub count: usize,
    pub cpu: KubeUtilization,
    pub memory: KubeUtilization,
    /// Every member sets CPU (memory) requests and limits. Datadog shows no utilization for a
    /// group with a container that lacks them.
    pub cpu_complete: bool,
    pub memory_complete: bool,
}

#[derive(Serialize, ToSchema)]
pub struct KubeKindCount {
    pub kind: KubeKind,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubeSearchPage {
    pub items: Vec<KubeResource>,
    pub total: usize,
    pub groups: Option<Vec<KubeGroup>>,
    pub facets: Vec<KubeFacet>,
    /// Objects of each kind matching the query.
    pub kinds: Vec<KubeKindCount>,
    /// Every tag key seen on this kind, for the group-by menu.
    pub tag_keys: Vec<String>,
}

fn sort_key(r: &KubeResource, field: &str) -> SortVal {
    let pct = |u: &Option<KubeUtilization>| u.as_ref().and_then(|u| u.usage).unwrap_or(-1.0);
    match field {
        "status" => SortVal::S(r.status.clone()),
        "age" => SortVal::N(-(r.created_at.unwrap_or(i64::MAX) as f64)),
        "restarts" => SortVal::N(r.restarts.unwrap_or(0) as f64),
        "cpu" => SortVal::N(pct(&r.cpu)),
        "memory" => SortVal::N(pct(&r.memory)),
        "cpu_requests" => SortVal::N(r.cpu.as_ref().and_then(|u| u.requests).unwrap_or(-1.0)),
        "memory_requests" => SortVal::N(r.memory.as_ref().and_then(|u| u.requests).unwrap_or(-1.0)),
        "namespace" => SortVal::S(r.namespace.clone().unwrap_or_default()),
        "cluster" => SortVal::S(r.cluster.clone()),
        "node" => SortVal::S(r.node.clone().unwrap_or_default()),
        "pods" => SortVal::N(r.pods.unwrap_or(0) as f64),
        "desired" => SortVal::N(r.desired.unwrap_or(0) as f64),
        "type" => SortVal::S(r.service_type.clone().unwrap_or_default()),
        "user" => SortVal::S(r.user.clone().unwrap_or_default()),
        _ => SortVal::S(r.name.clone()),
    }
}

#[derive(PartialEq, PartialOrd)]
enum SortVal {
    S(String),
    N(f64),
}

fn group_values(r: &KubeResource, keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| r.values(k).first().map(|v| v.to_string()).unwrap_or_else(|| format!("no {k}"))).collect()
}

fn group_of(r: &KubeResource, keys: &[&str]) -> String {
    group_values(r, keys).join(", ")
}

/// Requests and limits are set on every container (pods) or member (`memory` false = CPU).
fn complete(r: &KubeResource, memory: bool) -> bool {
    let set = |u: &KubeUtilization| u.requests.is_some() && u.limits.is_some();
    match r.kind {
        Some(KubeKind::Pods) => r.containers.iter().filter(|c| !c.init).all(|c| set(if memory { &c.memory } else { &c.cpu })),
        _ => (if memory { r.memory.as_ref() } else { r.cpu.as_ref() }).is_some_and(set),
    }
}

#[utoipa::path(post, path = "/api/v1/orchestration/search", operation_id = "search_orchestration", request_body = KubeSearchRequest, responses((status = 200, body = KubeSearchPage)), tag = "orchestration")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<KubeSearchRequest>) -> ApiResult<KubeSearchPage> {
    let all = build(&load(&st, &org).await?);
    let terms = parse_query(&req.query);
    // Counts of every kind under the same filter (the Overview's cluster / namespace cards).
    let kinds = KubeKind::ALL.into_iter().map(|k| KubeKindCount { kind: k, count: all.iter().filter(|r| r.kind == Some(k) && r.matches(&terms)).count() }).collect();
    let of_kind: Vec<KubeResource> = all.into_iter().filter(|r| r.kind == Some(req.kind)).collect();

    // KubeFacet counts honour every filter except the facet's own.
    let mut keys = if req.kind.is_ecs() || req.kind == KubeKind::Processes {
        vec![]
    } else {
        vec!["kube_cluster_name"]
    };
    if req.kind.namespaced() {
        keys.push("kube_namespace");
    }
    if req.kind != KubeKind::Processes {
        keys.push("status");
    }
    keys.extend(req.kind.facets());
    let facets = keys
        .into_iter()
        .map(|key| {
            let others: Vec<Term> = terms.iter().filter(|t| !matches!(t, Term::Tag { key: k, .. } if k == key)).cloned().collect();
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for r in of_kind.iter().filter(|r| r.matches(&others)) {
                for v in r.values(key).into_iter().collect::<HashSet<_>>() {
                    *counts.entry(v.to_string()).or_default() += 1;
                }
            }
            let mut values: Vec<KubeFacetValue> = counts.into_iter().map(|(value, count)| KubeFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            values.truncate(50);
            KubeFacet { key: key.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect();
    let mut tag_keys: Vec<String> = of_kind.iter().flat_map(|r| r.tags.iter().filter_map(|t| t.split_once(':').map(|(k, _)| k.to_string()))).collect::<HashSet<_>>().into_iter().collect();
    tag_keys.sort();

    let mut rows: Vec<KubeResource> = of_kind.into_iter().filter(|r| r.matches(&terms)).collect();
    let field = req.sort.as_deref().unwrap_or("name");
    let cmp = |a: &KubeResource, b: &KubeResource| {
        let o = sort_key(a, field).partial_cmp(&sort_key(b, field)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.name.cmp(&b.name));
        if req.desc {
            o.reverse()
        } else {
            o
        }
    };
    let total = rows.len();
    let limit = req.limit.clamp(1, 1000);
    let groups = match req.group_by.as_deref().filter(|g| !g.is_empty()) {
        Some(g) => {
            let g: Vec<&str> = g.split(',').map(str::trim).filter(|k| !k.is_empty()).collect();
            let g = g.as_slice();
            let mut groups: BTreeMap<String, KubeGroup> = BTreeMap::new();
            for r in &rows {
                let key = group_of(r, g);
                let e = groups.entry(key.clone()).or_insert_with(|| KubeGroup {
                    key,
                    values: group_values(r, g),
                    count: 0,
                    cpu: KubeUtilization::default(),
                    memory: KubeUtilization::default(),
                    cpu_complete: true,
                    memory_complete: true,
                });
                e.count += 1;
                e.cpu_complete &= complete(r, false);
                e.memory_complete &= complete(r, true);
                e.cpu.add(r.cpu.as_ref().unwrap_or(&KubeUtilization::default()));
                e.memory.add(r.memory.as_ref().unwrap_or(&KubeUtilization::default()));
            }
            let mut groups: Vec<KubeGroup> = groups.into_values().collect();
            groups.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
            let rank: HashMap<String, usize> = groups.iter().enumerate().map(|(i, g)| (g.key.clone(), i)).collect();
            rows.sort_by(|a, b| rank[&group_of(a, g)].cmp(&rank[&group_of(b, g)]).then_with(|| cmp(a, b)));
            Some(groups)
        }
        None => {
            rows.sort_by(cmp);
            None
        }
    };
    let items = rows.into_iter().skip(req.offset).take(limit).collect();
    Ok(Json(KubeSearchPage { items, total, groups, facets, kinds, tag_keys }))
}

// ================================================================ side panel

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubeEvent {
    pub timestamp: i64,
    /// Normal / Warning
    pub event_type: String,
    pub reason: String,
    pub message: String,
    pub object_kind: String,
    pub object_name: String,
    pub count: u32,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubeDetail {
    pub resource: KubeResource,
    /// The object as the API server returned it, without `managedFields`.
    #[schema(value_type = Object)]
    pub manifest: Value,
    /// Owners up the chain, then pods, services and other objects related to this one.
    pub related: Vec<KubeResource>,
    pub events: Vec<KubeEvent>,
}

#[utoipa::path(get, path = "/api/v1/orchestration/resources/{uid}", operation_id = "get_orchestration_resource", params(("uid" = String, Path)), responses((status = 200, body = KubeDetail), (status = 404, body = crate::error::ErrorBody)), tag = "orchestration")]
pub async fn detail(State(st): State<AppState>, OrgId(org): OrgId, Path(uid): Path<String>) -> ApiResult<KubeDetail> {
    let inv = load(&st, &org).await?;
    let rows = build(&inv);
    let r = rows.iter().find(|r| r.uid == uid).cloned().ok_or_else(|| ApiError::NotFound("Kubernetes resource not found".into()))?;
    let mut manifest = inv.objects.iter().find(|o| o.3.pointer("/metadata/uid").and_then(Value::as_str) == Some(uid.as_str())).map(|o| o.3.clone()).unwrap_or(Value::Null);
    if let Some(m) = manifest.get_mut("metadata").and_then(Value::as_object_mut) {
        m.remove("managedFields");
    }
    let related = related(&rows, &r);

    // Events about the object and, for workloads and nodes, about their pods.
    let mut uids: Vec<String> = vec![r.uid.clone()];
    if matches!(r.kind, Some(KubeKind::Deployments | KubeKind::ReplicaSets | KubeKind::DaemonSets | KubeKind::StatefulSets | KubeKind::Jobs | KubeKind::CronJobs)) {
        uids.extend(related.iter().filter(|x| x.kind != Some(KubeKind::Services)).map(|x| x.uid.clone()));
    }
    #[derive(clickhouse::Row, Deserialize)]
    struct Ev {
        ts: i64,
        typ: String,
        reason: String,
        note: String,
        kind: String,
        name: String,
        count: u32,
    }
    let events = if r.kind == Some(KubeKind::Clusters) {
        st.ch(&org)
            .query(
                "SELECT toUnixTimestamp64Milli(Timestamp) AS ts, Type AS typ, Reason AS reason, Note AS note, RegardingKind AS kind, RegardingName AS name, Count AS count \
                 FROM otel.k8s_events WHERE OrgId = {oc_org:String} AND Cluster = ? ORDER BY Timestamp DESC LIMIT 200",
            )
            .bind(&r.cluster)
            .fetch_all::<Ev>()
            .await?
    } else {
        st.ch(&org)
            .query(
                "SELECT toUnixTimestamp64Milli(Timestamp) AS ts, Type AS typ, Reason AS reason, Note AS note, RegardingKind AS kind, RegardingName AS name, Count AS count \
                 FROM otel.k8s_events WHERE OrgId = {oc_org:String} AND Cluster = ? AND (RegardingUid IN ? OR (RegardingKind = ? AND RegardingName = ? AND RegardingNamespace = ?)) \
                 ORDER BY Timestamp DESC LIMIT 200",
            )
            .bind(&r.cluster)
            .bind(&uids)
            .bind(r.kind.and_then(KubeKind::api_kind).unwrap_or_default())
            .bind(&r.name)
            .bind(r.namespace.clone().unwrap_or_default())
            .fetch_all::<Ev>()
            .await?
    };
    let events = events
        .into_iter()
        .map(|e| KubeEvent { timestamp: e.ts, event_type: e.typ, reason: e.reason, message: e.note, object_kind: e.kind, object_name: e.name, count: e.count })
        .collect();
    Ok(Json(KubeDetail { resource: r, manifest, related, events }))
}

/// Datadog's "Related Resources": the owner chain, owned objects and pods, and services that select them.
fn related(rows: &[KubeResource], r: &KubeResource) -> Vec<KubeResource> {
    let same_cluster = |x: &&KubeResource| x.cluster == r.cluster && x.uid != r.uid;
    let mut out: Vec<KubeResource> = Vec::new();
    let push = |x: &KubeResource, out: &mut Vec<KubeResource>| {
        if !out.iter().any(|o| o.uid == x.uid) {
            out.push(x.clone());
        }
    };
    // Owners.
    let mut cur = r.owner.clone();
    while let Some(o) = cur {
        match rows.iter().find(|x| x.uid == o.uid) {
            Some(x) => {
                push(x, &mut out);
                cur = x.owner.clone();
            }
            None => break,
        }
    }
    // Descendants (two levels: deployment → replicasets → pods, cronjob → jobs → pods).
    let children = |uid: &str| rows.iter().filter(|x| x.owner.as_ref().is_some_and(|o| o.uid == uid)).cloned().collect::<Vec<_>>();
    for c in children(&r.uid) {
        for g in children(&c.uid) {
            push(&g, &mut out);
        }
        push(&c, &mut out);
    }
    match r.kind {
        Some(KubeKind::Nodes) => rows.iter().filter(same_cluster).filter(|x| x.kind == Some(KubeKind::Pods) && x.node.as_deref() == Some(r.name.as_str())).for_each(|x| push(x, &mut out)),
        Some(KubeKind::Namespaces) => rows
            .iter()
            .filter(same_cluster)
            .filter(|x| x.namespace.as_deref() == Some(r.name.as_str()) && matches!(x.kind, Some(KubeKind::Deployments | KubeKind::DaemonSets | KubeKind::StatefulSets | KubeKind::CronJobs | KubeKind::Services)))
            .for_each(|x| push(x, &mut out)),
        Some(KubeKind::Clusters) => rows.iter().filter(same_cluster).filter(|x| x.kind == Some(KubeKind::Nodes)).for_each(|x| push(x, &mut out)),
        // ECS: a definition's, service's or cluster's tasks; an image's containers.
        Some(KubeKind::EcsTaskDefinitions) => rows.iter().filter(|x| x.kind == Some(KubeKind::EcsTasks) && x.task_definition.as_deref() == Some(r.name.as_str())).for_each(|x| push(x, &mut out)),
        Some(KubeKind::EcsServices) => rows
            .iter()
            .filter(same_cluster)
            .filter(|x| x.kind == Some(KubeKind::EcsTasks) && x.values("ecs_service").contains(&r.name.as_str()))
            .for_each(|x| push(x, &mut out)),
        Some(KubeKind::EcsClusters) => rows.iter().filter(|x| x.cluster == r.cluster && matches!(x.kind, Some(KubeKind::EcsTasks | KubeKind::EcsServices))).for_each(|x| push(x, &mut out)),
        Some(KubeKind::ContainerImages) => {
            let tag = r.image_tag.clone().unwrap_or_default();
            rows.iter()
                .filter(|x| x.kind == Some(KubeKind::Containers) && x.images.first().is_some_and(|i| image_parts(i).0 == r.name && image_parts(i).2 == tag))
                .for_each(|x| push(x, &mut out))
        }
        Some(KubeKind::Services) => {
            let sel: Vec<(String, String)> = r.selector.iter().flat_map(|s| s.split(',')).filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
            if !sel.is_empty() {
                rows.iter()
                    .filter(same_cluster)
                    .filter(|x| x.kind == Some(KubeKind::Pods) && x.namespace == r.namespace && sel.iter().all(|(k, v)| x.labels.get(k) == Some(v)))
                    .for_each(|x| push(x, &mut out));
            }
        }
        _ => {}
    }
    if r.kind == Some(KubeKind::EcsTasks) {
        let svc = r.values("ecs_service").first().map(|v| v.to_string());
        rows.iter()
            .filter(|x| {
                (x.kind == Some(KubeKind::EcsTaskDefinitions) && Some(&x.name) == r.task_definition.as_ref())
                    || (x.kind == Some(KubeKind::EcsServices) && x.cluster == r.cluster && Some(&x.name) == svc.as_ref())
                    || (x.kind == Some(KubeKind::EcsClusters) && x.cluster == r.cluster)
            })
            .for_each(|x| push(x, &mut out));
    }
    // Services selecting this pod or the pods of this workload.
    let pods: Vec<KubeResource> = if r.kind == Some(KubeKind::Pods) { vec![r.clone()] } else { out.iter().filter(|x| x.kind == Some(KubeKind::Pods)).cloned().collect() };
    if r.kind != Some(KubeKind::Services) && r.kind != Some(KubeKind::Nodes) {
        for svc in rows.iter().filter(same_cluster).filter(|x| x.kind == Some(KubeKind::Services)) {
            let sel: Vec<(&str, &str)> = svc.selector.iter().flat_map(|s| s.split(',')).filter_map(|kv| kv.split_once('=')).collect();
            if !sel.is_empty() && pods.iter().any(|p| p.namespace == svc.namespace && sel.iter().all(|(k, v)| p.labels.get(*k).map(String::as_str) == Some(*v))) {
                push(svc, &mut out);
            }
        }
    }
    out.truncate(500);
    out
}

/// The node named like a host and the pods running on it, for the host panel.
pub async fn host_inventory(st: &AppState, org: &str, host: &str) -> Result<(Option<KubeResource>, Vec<KubeResource>), ApiError> {
    let rows = build(&load(st, org).await?);
    let node = rows.iter().find(|r| r.kind == Some(KubeKind::Nodes) && r.name == host).cloned();
    Ok((node, rows.into_iter().filter(|r| r.kind == Some(KubeKind::Pods) && r.node.as_deref() == Some(host)).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datadog_usage_without_a_cluster_matches_a_unique_pod() {
        let pods = [("prod", "shop", "web-1", "uid-a"), ("stage", "shop", "web-1", "uid-b"), ("prod", "shop", "api-1", "uid-c")];
        let index = PodIndex::new(pods.iter().copied());
        assert_eq!(index.uid("prod", "shop", "web-1"), Some("uid-a"));
        // No cluster on the metric (an Agent without a cluster name): only an unambiguous pod matches.
        assert_eq!(index.uid("", "shop", "api-1"), Some("uid-c"));
        assert_eq!(index.uid("", "shop", "web-1"), None);
        assert_eq!(index.uid("prod", "shop", "gone"), None);
    }

    #[test]
    fn datadog_hosts_map_to_their_kubernetes_node() {
        let nodes = HashMap::from([("aks-infra-1-vmss000016", "aks-dev"), ("worker-1", "prod")]);
        // The Agent names a Kubernetes host `<node>-<cluster>`.
        assert_eq!(node_of_host("aks-infra-1-vmss000016-aks-dev", "", &nodes), Some(("aks-dev", "aks-infra-1-vmss000016")));
        assert_eq!(node_of_host("aks-infra-1-vmss000016-aks-dev", "aks-dev", &nodes), Some(("aks-dev", "aks-infra-1-vmss000016")));
        assert_eq!(node_of_host("worker-1", "", &nodes), Some(("prod", "worker-1")));
        assert_eq!(node_of_host("laptop", "", &nodes), None);
    }

    #[test]
    fn a_nodes_usage_sums_its_containers() {
        let key = |n: &str| ("aks-dev".to_string(), n.to_string());
        let totals = node_totals([(key("n1"), (0.25, 100.0)), (key("n1"), (0.5, -1.0)), (key("n2"), (-1.0, 50.0))]);
        assert_eq!(totals[&key("n1")], (0.75, 100.0));
        // -1: not measured; a node with nothing measured stays unmeasured.
        assert_eq!(totals[&key("n2")], (-1.0, 50.0));
    }

    #[test]
    fn pod_tags_carry_phase_and_readiness() {
        let status = serde_json::json!({ "phase": "Running", "conditions": [{ "type": "PodScheduled", "status": "True" }, { "type": "Ready", "status": "False" }] });
        assert_eq!(pod_tags(&status), vec!["pod_phase:running", "kube_condition_ready:false"]);
        assert_eq!(pod_tags(&serde_json::json!({ "phase": "Pending" })), vec!["pod_phase:pending"]);
    }

    #[test]
    fn quantities() {
        assert_eq!(parse_quantity("100m"), Some(0.1));
        assert_eq!(parse_quantity("2"), Some(2.0));
        assert_eq!(parse_quantity("64Mi"), Some(64.0 * 1024.0 * 1024.0));
        assert_eq!(parse_quantity("1G"), Some(1e9));
        assert_eq!(parse_quantity("129e6"), Some(129e6));
        assert_eq!(parse_quantity("lots"), None);
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-10-25T18:46:38Z"), Some(1_792_953_998_000));
        assert_eq!(parse_time("2026-10-25T18:46:38.123456Z"), Some(1_792_953_998_000));
        assert_eq!(parse_time("yesterday"), None);
    }

    #[test]
    fn images() {
        assert_eq!(image_parts("docker.io/library/busybox:1.36"), ("docker.io/library/busybox".into(), "busybox".into(), "1.36".into()));
        assert_eq!(image_parts("localhost:5000/shop/api"), ("localhost:5000/shop/api".into(), "api".into(), "latest".into()));
        assert_eq!(image_parts("nginx@sha256:abc"), ("nginx".into(), "nginx".into(), "latest".into()));
    }

    fn pod(status: Value) -> Value {
        serde_json::json!({ "kind": "Pod", "metadata": { "name": "p", "uid": "u" }, "spec": { "containers": [{ "name": "c" }] }, "status": status })
    }

    #[test]
    fn pod_statuses_like_kubectl() {
        let crash = pod(serde_json::json!({ "phase": "Running", "containerStatuses": [{ "name": "c", "ready": false, "state": { "waiting": { "reason": "CrashLoopBackOff" } } }] }));
        assert_eq!(pod_status(&crash), ("CrashLoopBackOff".into(), KubeHealth::Error));
        let ok = pod(serde_json::json!({ "phase": "Running", "containerStatuses": [{ "name": "c", "ready": true, "state": { "running": {} } }] }));
        assert_eq!(pod_status(&ok), ("Running".into(), KubeHealth::Ok));
        let done = pod(serde_json::json!({ "phase": "Succeeded", "containerStatuses": [{ "name": "c", "ready": false, "state": { "terminated": { "reason": "Completed" } } }] }));
        assert_eq!(pod_status(&done), ("Completed".into(), KubeHealth::Info));
        let oom = pod(serde_json::json!({ "phase": "Running", "containerStatuses": [{ "name": "c", "ready": false, "state": { "terminated": { "reason": "OOMKilled" } } }] }));
        assert_eq!(pod_status(&oom), ("OOMKilled".into(), KubeHealth::Error));
    }
}
