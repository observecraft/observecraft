//! Kubernetes → Remediation: the Deployments, DaemonSets and StatefulSets whose pods have a common
//! problem (crash loops, out-of-memory kills, image pulls, container configuration, scheduling,
//! readiness, restarts), one row per workload and problem, with how to fix it. Built from the
//! explorer's inventory (pod statuses and containers), so it is as live as the explorer.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::kubernetes::{resources, KubeKind, KubeResource};
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

/// Containers restarting more than this are a problem even when running.
const RESTARTS: u32 = 5;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    CrashLoopBackOff,
    OomKilled,
    ImagePull,
    ContainerConfig,
    Pending,
    FailedPods,
    NotReady,
    FrequentRestarts,
}

impl IssueType {
    pub fn label(self) -> &'static str {
        match self {
            IssueType::CrashLoopBackOff => "CrashLoopBackOff",
            IssueType::OomKilled => "OOMKilled",
            IssueType::ImagePull => "Image pull failure",
            IssueType::ContainerConfig => "Container configuration error",
            IssueType::Pending => "Pods stuck pending",
            IssueType::FailedPods => "Failed pods",
            IssueType::NotReady => "Failing readiness",
            IssueType::FrequentRestarts => "Frequent restarts",
        }
    }

    /// What to check first.
    pub fn remediation(self) -> &'static str {
        match self {
            IssueType::CrashLoopBackOff => "The container exits right after starting. Read its last logs (kubectl logs --previous), then check the command, configuration and the dependencies it needs at startup.",
            IssueType::OomKilled => "The container used more memory than its limit. Raise the memory limit (and request) to the peak it needs, or find what grows its memory.",
            IssueType::ImagePull => "The node can't pull the image. Check the image name and tag, that it exists in the registry, and the pod's imagePullSecrets.",
            IssueType::ContainerConfig => "The container can't be created. A referenced ConfigMap, Secret or volume is usually missing; kubectl describe pod names it.",
            IssueType::Pending => "No node can run the pods. Compare their requests, node selectors, affinities and tolerations with the nodes' free capacity, or add nodes.",
            IssueType::FailedPods => "Pods ended in Failed (often evicted for node pressure). Check the node's memory and disk pressure, and the pods' ephemeral storage.",
            IssueType::NotReady => "Containers run but fail their readiness probe, so they get no traffic. Check the probe's path, port and timeouts, and the service's dependencies.",
            IssueType::FrequentRestarts => "Containers keep restarting. Check their liveness probe and logs before the restarts, and whether they are killed for memory.",
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadProblem {
    pub cluster: String,
    pub namespace: String,
    /// Deployment, DaemonSet or StatefulSet.
    pub kind: String,
    pub name: String,
    pub uid: String,
    pub service: Option<String>,
    pub team: Option<String>,
    pub issue: IssueType,
    pub issue_label: String,
    pub remediation: String,
    pub pods_affected: u32,
    pub pods_total: u32,
    /// The oldest affected pod's creation (epoch ms).
    pub started: Option<i64>,
    /// Up to 5 affected pods.
    pub pods: Vec<String>,
}

/// The problem of one pod, if it has one (the worst first).
pub fn pod_issue(p: &KubeResource) -> Option<IssueType> {
    let regular = || p.containers.iter().filter(|c| !c.init);
    if p.containers.iter().any(|c| c.reason.as_deref() == Some("OOMKilled")) {
        return Some(IssueType::OomKilled);
    }
    Some(match p.status.as_str() {
        "Completed" | "Succeeded" | "Terminating" => return None,
        "CrashLoopBackOff" => IssueType::CrashLoopBackOff,
        // Between back-offs a crash-looping container shows as terminated with `Error`.
        "Error" if regular().any(|c| c.restarts > 0) => IssueType::CrashLoopBackOff,
        "ImagePullBackOff" | "ErrImagePull" | "InvalidImageName" => IssueType::ImagePull,
        "CreateContainerConfigError" | "CreateContainerError" | "RunContainerError" => IssueType::ContainerConfig,
        "Pending" | "ContainerCreating" | "PodInitializing" => IssueType::Pending,
        "Failed" | "Error" | "Evicted" | "Unknown" => IssueType::FailedPods,
        _ if regular().any(|c| !c.ready) => IssueType::NotReady,
        _ if regular().map(|c| c.restarts).max().unwrap_or(0) >= RESTARTS => IssueType::FrequentRestarts,
        _ => return None,
    })
}

/// The pods' tag naming their workload, per workload kind.
const WORKLOADS: [(KubeKind, &str, &str); 3] = [(KubeKind::Deployments, "Deployment", "kube_deployment"), (KubeKind::DaemonSets, "DaemonSet", "kube_daemon_set"), (KubeKind::StatefulSets, "StatefulSet", "kube_stateful_set")];

fn tag<'a>(r: &'a KubeResource, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    r.tags.iter().find_map(|t| t.strip_prefix(&prefix))
}

/// One row per workload and problem, the most affected first.
pub fn problems(rows: &[KubeResource]) -> Vec<WorkloadProblem> {
    // (cluster, namespace, kind, name) → (total pods, issue → affected pods)
    type Pods<'a> = (u32, BTreeMap<IssueType, Vec<&'a KubeResource>>);
    let mut by_workload: BTreeMap<(String, String, &'static str, String), Pods> = BTreeMap::new();
    for p in rows.iter().filter(|r| r.kind == Some(KubeKind::Pods)) {
        let Some((kind, name)) = WORKLOADS.iter().find_map(|(_, kind, key)| tag(p, key).map(|n| (*kind, n.to_string()))) else { continue };
        let entry = by_workload.entry((p.cluster.clone(), p.namespace.clone().unwrap_or_default(), kind, name)).or_default();
        entry.0 += 1;
        if let Some(issue) = pod_issue(p) {
            entry.1.entry(issue).or_default().push(p);
        }
    }
    let mut out: Vec<WorkloadProblem> = by_workload
        .into_iter()
        .flat_map(|((cluster, namespace, kind, name), (total, issues))| {
            let w = rows.iter().find(|r| WORKLOADS.iter().any(|(k, n, _)| r.kind == Some(*k) && *n == kind) && r.cluster == cluster && r.namespace.as_deref() == Some(namespace.as_str()) && r.name == name);
            issues.into_iter().map(move |(issue, pods)| {
                let first = pods[0];
                WorkloadProblem {
                    cluster: cluster.clone(),
                    namespace: namespace.clone(),
                    kind: kind.to_string(),
                    name: name.clone(),
                    uid: w.map(|w| w.uid.clone()).unwrap_or_default(),
                    service: tag(first, "service").map(str::to_string),
                    team: tag(first, "team").map(str::to_string),
                    issue,
                    issue_label: issue.label().into(),
                    remediation: issue.remediation().into(),
                    pods_affected: u32::try_from(pods.len()).unwrap_or(u32::MAX),
                    pods_total: total,
                    started: pods.iter().filter_map(|p| p.created_at).min(),
                    pods: pods.iter().take(5).map(|p| p.name.clone()).collect(),
                }
            })
        })
        .collect();
    out.sort_by(|a, b| b.pods_affected.cmp(&a.pods_affected).then(a.issue.cmp(&b.issue)).then(a.name.cmp(&b.name)));
    out
}

#[utoipa::path(get, path = "/api/v1/kubernetes/remediation", operation_id = "list_kubernetes_workload_problems", responses((status = 200, body = [WorkloadProblem])), tag = "orchestration")]
pub async fn list(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<WorkloadProblem>> {
    Ok(Json(problems(&resources(&st, &org).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::kubernetes::{KubeContainer, KubeUtilization};

    fn container(state: &str, reason: Option<&str>, ready: bool, restarts: u32) -> KubeContainer {
        KubeContainer {
            name: "app".into(),
            image: "registry.example.com/billing:2.4.0".into(),
            ready,
            restarts,
            state: state.into(),
            reason: reason.map(str::to_string),
            started_at: None,
            init: false,
            cpu: KubeUtilization::default(),
            memory: KubeUtilization::default(),
        }
    }

    fn pod(workload: (&str, &str), i: u32, status: &str, c: KubeContainer) -> KubeResource {
        KubeResource {
            uid: format!("{}-{i}", workload.1),
            kind: Some(KubeKind::Pods),
            cluster: "shop-prod".into(),
            namespace: Some("payments".into()),
            name: format!("{}-7d9f8c6b5-x{i}abc", workload.1),
            status: status.into(),
            created_at: Some(1_000 + i64::from(i)),
            restarts: Some(c.restarts),
            tags: vec![format!("{}:{}", workload.0, workload.1), format!("service:{}", workload.1), "team:payments".into()],
            containers: vec![c],
            ..Default::default()
        }
    }

    fn workload(kind: KubeKind, name: &str) -> KubeResource {
        KubeResource { uid: format!("uid-{name}"), kind: Some(kind), cluster: "shop-prod".into(), namespace: Some("payments".into()), name: name.into(), ..Default::default() }
    }

    #[test]
    fn pod_states_map_to_issues() {
        let w = ("kube_deployment", "billing");
        assert_eq!(pod_issue(&pod(w, 0, "CrashLoopBackOff", container("waiting", Some("CrashLoopBackOff"), false, 12))), Some(IssueType::CrashLoopBackOff));
        assert_eq!(pod_issue(&pod(w, 0, "OOMKilled", container("terminated", Some("OOMKilled"), false, 3))), Some(IssueType::OomKilled));
        assert_eq!(pod_issue(&pod(w, 0, "ErrImagePull", container("waiting", Some("ErrImagePull"), false, 0))), Some(IssueType::ImagePull));
        assert_eq!(pod_issue(&pod(w, 0, "ImagePullBackOff", container("waiting", Some("ImagePullBackOff"), false, 0))), Some(IssueType::ImagePull));
        assert_eq!(pod_issue(&pod(w, 0, "CreateContainerConfigError", container("waiting", Some("CreateContainerConfigError"), false, 0))), Some(IssueType::ContainerConfig));
        assert_eq!(pod_issue(&pod(w, 0, "Pending", container("waiting", Some("ContainerCreating"), false, 0))), Some(IssueType::Pending));
        assert_eq!(pod_issue(&pod(w, 0, "Failed", container("terminated", Some("Error"), false, 0))), Some(IssueType::FailedPods));
        // Between back-offs a crash-looping container is terminated with `Error`: its restarts tell them apart.
        assert_eq!(pod_issue(&pod(w, 0, "Error", container("terminated", Some("Error"), false, 3))), Some(IssueType::CrashLoopBackOff));
        assert_eq!(pod_issue(&pod(w, 0, "Error", container("terminated", Some("Error"), false, 0))), Some(IssueType::FailedPods));
        assert_eq!(pod_issue(&pod(w, 0, "Running", container("running", None, false, 0))), Some(IssueType::NotReady));
        assert_eq!(pod_issue(&pod(w, 0, "Running", container("running", None, true, 7))), Some(IssueType::FrequentRestarts));
        assert_eq!(pod_issue(&pod(w, 0, "Running", container("running", None, true, 1))), None);
        assert_eq!(pod_issue(&pod(w, 0, "Completed", container("terminated", Some("Completed"), false, 0))), None);
    }

    #[test]
    fn problems_group_pods_by_workload_and_issue() {
        let billing = ("kube_deployment", "billing");
        let agent = ("kube_daemon_set", "log-shipper");
        let rows = vec![
            workload(KubeKind::Deployments, "billing"),
            workload(KubeKind::DaemonSets, "log-shipper"),
            pod(billing, 0, "CrashLoopBackOff", container("waiting", Some("CrashLoopBackOff"), false, 12)),
            pod(billing, 1, "CrashLoopBackOff", container("waiting", Some("CrashLoopBackOff"), false, 9)),
            pod(billing, 2, "Running", container("running", None, true, 0)),
            pod(agent, 0, "Running", container("running", None, false, 0)),
            pod(agent, 1, "Running", container("running", None, true, 0)),
            pod(agent, 2, "Running", container("running", None, true, 0)),
            pod(agent, 3, "Running", container("running", None, true, 0)),
        ];
        let got = problems(&rows);
        assert_eq!(got.len(), 2);
        let first = &got[0];
        assert_eq!((first.kind.as_str(), first.name.as_str(), first.issue, first.pods_affected, first.pods_total), ("Deployment", "billing", IssueType::CrashLoopBackOff, 2, 3));
        assert_eq!((first.started, first.service.as_deref(), first.team.as_deref()), (Some(1_000), Some("billing"), Some("payments")));
        assert_eq!(first.pods.len(), 2);
        assert_eq!(first.issue_label, "CrashLoopBackOff");
        assert!(first.remediation.contains("kubectl logs --previous"));
        assert_eq!((got[1].kind.as_str(), got[1].issue, got[1].pods_affected, got[1].pods_total), ("DaemonSet", IssueType::NotReady, 1, 4));
    }
}
