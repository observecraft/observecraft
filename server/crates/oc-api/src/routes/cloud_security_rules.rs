//! Cloud Security's misconfiguration rules: checks of the resource catalog's hosts, Kubernetes
//! workloads and services, on their catalog attributes and (for Kubernetes) the objects' manifests.
//! The rules are our own, written after the hardening advice of the Kubernetes Pod Security Standards
//! and common host hygiene. Pure functions; [`super::cloud_security`] evaluates them on read.

use serde_json::Value;

use super::catalog::{CatalogResource, CatalogType};

/// What a rule looks at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Pods, Deployments, StatefulSets, DaemonSets, Jobs and CronJobs (by their pod template).
    Workload,
    Service,
    Host,
}

pub struct MisconfigRule {
    /// Stable id (`ocs-k8s-001`).
    pub id: &'static str,
    pub name: &'static str,
    /// critical | high | medium | low
    pub severity: &'static str,
    pub target: Target,
    pub description: &'static str,
    pub remediation: &'static str,
    /// Frameworks and controls the rule maps to.
    pub frameworks: &'static [&'static str],
    /// The failure (what is wrong, e.g. the containers concerned), or None when the resource passes.
    pub check: fn(&Subject) -> Option<String>,
}

/// A resource with its manifest (Kubernetes objects).
pub struct Subject<'a> {
    pub resource: &'a CatalogResource,
    pub manifest: Option<&'a Value>,
}

/// The pod spec of a workload's manifest.
pub fn pod_spec(kind: CatalogType, m: &Value) -> Option<&Value> {
    match kind {
        CatalogType::KubePod => m.get("spec"),
        CatalogType::KubeCronJob => m.pointer("/spec/jobTemplate/spec/template/spec"),
        CatalogType::KubeDeployment | CatalogType::KubeStatefulSet | CatalogType::KubeDaemonSet | CatalogType::KubeJob => m.pointer("/spec/template/spec"),
        _ => None,
    }
}

/// Every container of a pod spec (init containers included), with its name.
fn containers(spec: &Value) -> Vec<(&str, &Value)> {
    ["initContainers", "containers"]
        .iter()
        .filter_map(|k| spec.get(*k).and_then(Value::as_array))
        .flatten()
        .map(|c| (c.get("name").and_then(Value::as_str).unwrap_or("?"), c))
        .collect()
}

/// The pod spec of a workload subject.
fn spec<'a>(s: &Subject<'a>) -> Option<&'a Value> {
    pod_spec(s.resource.resource_type, s.manifest?)
}

/// Lists the containers for which `bad` holds, as the failure.
fn failing(s: &Subject, what: &str, bad: impl Fn(&Value, &Value) -> bool) -> Option<String> {
    let spec = spec(s)?;
    let names: Vec<&str> = containers(spec).into_iter().filter(|(_, c)| bad(spec, c)).map(|(n, _)| n).collect();
    (!names.is_empty()).then(|| format!("{what}: {}", names.join(", ")))
}

fn flag(v: &Value, ptr: &str) -> Option<bool> {
    v.pointer(ptr).and_then(Value::as_bool)
}

fn privileged(s: &Subject) -> Option<String> {
    failing(s, "privileged containers", |_, c| flag(c, "/securityContext/privileged") == Some(true))
}

fn privilege_escalation(s: &Subject) -> Option<String> {
    failing(s, "containers that may gain privileges", |_, c| flag(c, "/securityContext/allowPrivilegeEscalation") != Some(false) && flag(c, "/securityContext/privileged") != Some(true))
}

fn runs_as_root(s: &Subject) -> Option<String> {
    failing(s, "containers that may run as root", |pod, c| {
        let non_root = flag(c, "/securityContext/runAsNonRoot").or_else(|| flag(pod, "/securityContext/runAsNonRoot")) == Some(true);
        let user = c.pointer("/securityContext/runAsUser").or_else(|| pod.pointer("/securityContext/runAsUser")).and_then(Value::as_i64);
        match user {
            Some(0) => true,
            Some(_) => false,
            None => !non_root,
        }
    })
}

fn host_namespaces(s: &Subject) -> Option<String> {
    let spec = spec(s)?;
    let shared: Vec<&str> = [("hostNetwork", "network"), ("hostPID", "PID"), ("hostIPC", "IPC")].iter().filter(|(k, _)| spec.get(*k).and_then(Value::as_bool) == Some(true)).map(|(_, n)| *n).collect();
    (!shared.is_empty()).then(|| format!("shares the host's {} namespace{}", shared.join(", "), if shared.len() > 1 { "s" } else { "" }))
}

fn host_path(s: &Subject) -> Option<String> {
    let spec = spec(s)?;
    let paths: Vec<String> = spec.get("volumes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.pointer("/hostPath/path").and_then(Value::as_str).map(str::to_string)).collect();
    (!paths.is_empty()).then(|| format!("mounts host paths: {}", paths.join(", ")))
}

const DANGEROUS_CAPS: [&str; 6] = ["ALL", "SYS_ADMIN", "NET_ADMIN", "SYS_PTRACE", "SYS_MODULE", "NET_RAW"];

fn capabilities(s: &Subject) -> Option<String> {
    failing(s, "containers adding dangerous capabilities", |_, c| {
        c.pointer("/securityContext/capabilities/add").and_then(Value::as_array).is_some_and(|a| a.iter().filter_map(Value::as_str).any(|cap| DANGEROUS_CAPS.contains(&cap.trim_start_matches("CAP_"))))
    })
}

fn writable_root(s: &Subject) -> Option<String> {
    failing(s, "containers with a writable root file system", |_, c| flag(c, "/securityContext/readOnlyRootFilesystem") != Some(true))
}

fn no_limits(s: &Subject) -> Option<String> {
    failing(s, "containers without memory or CPU limits", |_, c| c.pointer("/resources/limits/memory").is_none() || c.pointer("/resources/limits/cpu").is_none())
}

/// An image without a tag, or tagged `latest` (digests pin the image).
pub fn floating_image(image: &str) -> bool {
    if image.contains('@') {
        return false;
    }
    let last = image.rsplit('/').next().unwrap_or(image);
    match last.split_once(':') {
        None => true,
        Some((_, tag)) => tag == "latest",
    }
}

fn latest_tag(s: &Subject) -> Option<String> {
    failing(s, "containers with a floating image tag", |_, c| c.get("image").and_then(Value::as_str).is_some_and(floating_image))
}

fn token_automount(s: &Subject) -> Option<String> {
    let spec = spec(s)?;
    (spec.get("automountServiceAccountToken").and_then(Value::as_bool) != Some(false)).then(|| "the service account token is mounted into the pods".to_string())
}

fn default_namespace(s: &Subject) -> Option<String> {
    spec(s)?;
    (s.resource.namespace.as_deref() == Some("default")).then(|| "runs in the default namespace".to_string())
}

fn open_load_balancer(s: &Subject) -> Option<String> {
    let m = s.manifest?;
    let lb = m.pointer("/spec/type").and_then(Value::as_str) == Some("LoadBalancer");
    let ranges = m.pointer("/spec/loadBalancerSourceRanges").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    let internal = m.pointer("/metadata/annotations").and_then(Value::as_object).is_some_and(|a| a.iter().any(|(k, v)| k.contains("internal") && v.as_str().is_some_and(|v| v == "true" || v.contains("internal"))));
    (lb && !ranges && !internal).then(|| "load balancer reachable from any address".to_string())
}

fn node_port(s: &Subject) -> Option<String> {
    (s.manifest?.pointer("/spec/type").and_then(Value::as_str) == Some("NodePort")).then(|| "exposes a port on every node".to_string())
}

/// Distributions past their vendor's standard support.
const EOL: [&str; 8] = ["ubuntu 16.04", "ubuntu 18.04", "debian 9", "debian 10", "centos 7", "centos 8", "amazon linux ami", "rhel 7"];

fn os_end_of_life(s: &Subject) -> Option<String> {
    let os = [s.resource.attributes.get("distro"), s.resource.attributes.get("os_description")].into_iter().flatten().map(|v| v.to_lowercase()).collect::<Vec<_>>().join(" ");
    EOL.iter().find(|e| os.contains(*e)).map(|e| format!("{e} no longer receives security updates"))
}

fn agent_outdated(s: &Subject) -> Option<String> {
    let v = s.resource.attributes.get("agent_version")?;
    let parts: Vec<u64> = v.split(['.', '-']).take(2).filter_map(|p| p.parse().ok()).collect();
    match parts.as_slice() {
        [major, minor] if (*major, *minor) < (7, 50) => Some(format!("Datadog Agent {v} is older than 7.50")),
        _ => None,
    }
}

pub const RULES: &[MisconfigRule] = &[
    MisconfigRule {
        id: "ocs-k8s-001",
        name: "Containers should not run privileged",
        severity: "critical",
        target: Target::Workload,
        description: "A privileged container has every capability of the host and can escape to the node.",
        remediation: "Remove `securityContext.privileged: true`; grant only the capabilities the container needs.",
        frameworks: &["pod-security-standards:baseline", "mitre:T1611-escape-to-host"],
        check: privileged,
    },
    MisconfigRule {
        id: "ocs-k8s-002",
        name: "Pods should not share the host's namespaces",
        severity: "high",
        target: Target::Workload,
        description: "Sharing the host network, PID or IPC namespace lets the pod see and reach the node's processes and traffic.",
        remediation: "Set `hostNetwork`, `hostPID` and `hostIPC` to false.",
        frameworks: &["pod-security-standards:baseline"],
        check: host_namespaces,
    },
    MisconfigRule {
        id: "ocs-k8s-003",
        name: "Containers should not add dangerous capabilities",
        severity: "high",
        target: Target::Workload,
        description: "Capabilities such as SYS_ADMIN or NET_ADMIN give a container control over the node's kernel or network.",
        remediation: "Drop ALL capabilities and add back only the ones the process needs (NET_BIND_SERVICE, …).",
        frameworks: &["pod-security-standards:baseline"],
        check: capabilities,
    },
    MisconfigRule {
        id: "ocs-k8s-004",
        name: "Pods should not mount host paths",
        severity: "medium",
        target: Target::Workload,
        description: "A hostPath volume exposes the node's file system to the pod.",
        remediation: "Use persistent volumes, ConfigMaps or emptyDir volumes instead of hostPath.",
        frameworks: &["pod-security-standards:baseline"],
        check: host_path,
    },
    MisconfigRule {
        id: "ocs-k8s-005",
        name: "Containers should not run as root",
        severity: "medium",
        target: Target::Workload,
        description: "A process running as root inside the container is root on the node if it escapes.",
        remediation: "Set `runAsNonRoot: true` and a non-zero `runAsUser` in the pod or container security context.",
        frameworks: &["pod-security-standards:restricted"],
        check: runs_as_root,
    },
    MisconfigRule {
        id: "ocs-k8s-006",
        name: "Containers should not allow privilege escalation",
        severity: "medium",
        target: Target::Workload,
        description: "Without `allowPrivilegeEscalation: false`, a process can gain more privileges than its parent (setuid binaries).",
        remediation: "Set `securityContext.allowPrivilegeEscalation: false` on every container.",
        frameworks: &["pod-security-standards:restricted"],
        check: privilege_escalation,
    },
    MisconfigRule {
        id: "ocs-k8s-007",
        name: "Container images should be pinned",
        severity: "medium",
        target: Target::Workload,
        description: "Images without a tag or tagged `latest` change under you: what runs is not what was reviewed and scanned.",
        remediation: "Reference images by a version tag or, better, by digest.",
        frameworks: &["supply-chain"],
        check: latest_tag,
    },
    MisconfigRule {
        id: "ocs-k8s-008",
        name: "Load balancers should restrict source addresses",
        severity: "medium",
        target: Target::Service,
        description: "A LoadBalancer service without source ranges is reachable from the whole internet.",
        remediation: "Set `loadBalancerSourceRanges`, make the load balancer internal, or put it behind an ingress with authentication.",
        frameworks: &["network-exposure"],
        check: open_load_balancer,
    },
    MisconfigRule {
        id: "ocs-k8s-009",
        name: "Containers should have a read-only root file system",
        severity: "low",
        target: Target::Workload,
        description: "A writable root file system lets an attacker drop tools and persist changes in the container.",
        remediation: "Set `readOnlyRootFilesystem: true` and mount writable emptyDir volumes where the process writes.",
        frameworks: &["pod-security-standards:restricted"],
        check: writable_root,
    },
    MisconfigRule {
        id: "ocs-k8s-010",
        name: "Containers should have resource limits",
        severity: "low",
        target: Target::Workload,
        description: "Without memory and CPU limits, one container can starve the others on its node.",
        remediation: "Set `resources.limits.memory` and `resources.limits.cpu` on every container.",
        frameworks: &["availability"],
        check: no_limits,
    },
    MisconfigRule {
        id: "ocs-k8s-011",
        name: "Pods should not mount the service account token unless they need it",
        severity: "low",
        target: Target::Workload,
        description: "A mounted token lets anyone with code execution in the pod call the Kubernetes API as its service account.",
        remediation: "Set `automountServiceAccountToken: false` on pods that don't call the API.",
        frameworks: &["kubernetes-hardening"],
        check: token_automount,
    },
    MisconfigRule {
        id: "ocs-k8s-012",
        name: "Workloads should not run in the default namespace",
        severity: "low",
        target: Target::Workload,
        description: "The default namespace usually has no network policies, quotas or RBAC boundaries of its own.",
        remediation: "Move the workload to a namespace of its own.",
        frameworks: &["kubernetes-hardening"],
        check: default_namespace,
    },
    MisconfigRule {
        id: "ocs-k8s-013",
        name: "Services should not use NodePort",
        severity: "low",
        target: Target::Service,
        description: "A NodePort service opens the port on every node of the cluster.",
        remediation: "Use a ClusterIP service behind an ingress or an internal load balancer.",
        frameworks: &["network-exposure"],
        check: node_port,
    },
    MisconfigRule {
        id: "ocs-host-001",
        name: "Hosts should run a supported operating system",
        severity: "high",
        target: Target::Host,
        description: "A distribution past its end of life no longer gets security fixes.",
        remediation: "Upgrade the host to a supported release of its distribution.",
        frameworks: &["patch-management"],
        check: os_end_of_life,
    },
    MisconfigRule {
        id: "ocs-host-002",
        name: "The Datadog Agent should be up to date",
        severity: "low",
        target: Target::Host,
        description: "Older Agents miss security fixes and the features Cloud Security relies on (host SBOMs).",
        remediation: "Upgrade the Agent to 7.50 or later.",
        frameworks: &["patch-management"],
        check: agent_outdated,
    },
];

/// What a resource is for the rules.
pub fn target_of(r: &CatalogResource) -> Option<Target> {
    match r.resource_type {
        CatalogType::Host => Some(Target::Host),
        CatalogType::KubeService => Some(Target::Service),
        // Pods and Jobs owned by another workload are checked through their owner.
        CatalogType::KubePod | CatalogType::KubeJob if r.attributes.contains_key("owner") => None,
        CatalogType::KubePod | CatalogType::KubeDeployment | CatalogType::KubeStatefulSet | CatalogType::KubeDaemonSet | CatalogType::KubeCronJob | CatalogType::KubeJob => Some(Target::Workload),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn resource(kind: CatalogType, ns: &str) -> CatalogResource {
        CatalogResource {
            id: "uid-1".into(),
            resource_type: kind,
            resource_category: super::super::catalog::CatalogCategory::Kubernetes,
            name: "checkout".into(),
            cloud: None,
            account: None,
            region: None,
            cluster: Some("shop-prod".into()),
            namespace: Some(ns.into()),
            env: Some("prod".into()),
            service: None,
            team: None,
            status: None,
            health: None,
            created_at: None,
            last_seen: 0,
            tags: vec![],
            attributes: BTreeMap::new(),
            kube_kind: None,
        }
    }

    fn rule(id: &str) -> &'static MisconfigRule {
        RULES.iter().find(|r| r.id == id).unwrap()
    }

    #[test]
    fn workload_rules_read_the_pod_template() {
        let m = json!({"spec": {"template": {"spec": {
            "hostNetwork": true,
            "volumes": [{"name": "docker", "hostPath": {"path": "/var/run/docker.sock"}}],
            "containers": [
                {"name": "app", "image": "registry.example.com/checkout:latest", "securityContext": {"privileged": true, "capabilities": {"add": ["SYS_ADMIN"]}}},
                {"name": "sidecar", "image": "registry.example.com/proxy:1.4.2@sha256:abc", "securityContext": {"runAsNonRoot": true, "runAsUser": 1000, "allowPrivilegeEscalation": false, "readOnlyRootFilesystem": true},
                 "resources": {"limits": {"cpu": "100m", "memory": "64Mi"}}}
            ]}}}});
        let r = resource(CatalogType::KubeDeployment, "default");
        let s = Subject { resource: &r, manifest: Some(&m) };
        let fail = |id: &str| (rule(id).check)(&s);
        assert_eq!(fail("ocs-k8s-001").as_deref(), Some("privileged containers: app"));
        assert!(fail("ocs-k8s-002").unwrap().contains("network"));
        assert!(fail("ocs-k8s-003").unwrap().ends_with("app"));
        assert!(fail("ocs-k8s-004").unwrap().contains("/var/run/docker.sock"));
        assert_eq!(fail("ocs-k8s-005").as_deref(), Some("containers that may run as root: app"));
        assert_eq!(fail("ocs-k8s-007").as_deref(), Some("containers with a floating image tag: app"));
        assert_eq!(fail("ocs-k8s-010").as_deref(), Some("containers without memory or CPU limits: app"));
        assert!(fail("ocs-k8s-012").is_some());
        // Privileged containers are reported once, not also as escalating.
        assert!(fail("ocs-k8s-006").is_none());
    }

    #[test]
    fn floating_tags_and_services() {
        assert!(floating_image("nginx"));
        assert!(floating_image("registry.example.com:5000/team/app:latest"));
        assert!(!floating_image("app:1.2.3"));
        assert!(!floating_image("app@sha256:abc"));
        let svc = resource(CatalogType::KubeService, "shop");
        let lb = json!({"spec": {"type": "LoadBalancer"}});
        assert!((rule("ocs-k8s-008").check)(&Subject { resource: &svc, manifest: Some(&lb) }).is_some());
        let restricted = json!({"spec": {"type": "LoadBalancer", "loadBalancerSourceRanges": ["192.0.2.0/24"]}});
        assert!((rule("ocs-k8s-008").check)(&Subject { resource: &svc, manifest: Some(&restricted) }).is_none());
    }

    #[test]
    fn host_rules_read_attributes() {
        let mut h = resource(CatalogType::Host, "");
        h.attributes.insert("distro".into(), "Ubuntu 18.04".into());
        h.attributes.insert("agent_version".into(), "7.45.1".into());
        let s = Subject { resource: &h, manifest: None };
        assert!((rule("ocs-host-001").check)(&s).is_some());
        assert!((rule("ocs-host-002").check)(&s).is_some());
        h.attributes.insert("agent_version".into(), "7.58.0".into());
        assert!((rule("ocs-host-002").check)(&Subject { resource: &h, manifest: None }).is_none());
    }

    #[test]
    fn owned_pods_are_checked_through_their_owner() {
        let mut p = resource(CatalogType::KubePod, "shop");
        assert_eq!(target_of(&p), Some(Target::Workload));
        p.attributes.insert("owner".into(), "ReplicaSet/checkout-7d9".into());
        assert_eq!(target_of(&p), None);
        let ids: std::collections::HashSet<_> = RULES.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), RULES.len());
    }
}
