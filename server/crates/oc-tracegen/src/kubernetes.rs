//! Fictional Kubernetes telemetry through the same authenticated OTLP intake as real collectors.
//! Resource names agree with `hosts`; object counts and resource usage come from the same pod model.
//! Compatibility metric names are intentional: the bundled integration dashboards query these names.

use serde_json::{json, Value};

use crate::{attrs_json, hosts::HOSTS};

const GIB: f64 = 1_073_741_824.0;
const SECOND: u64 = 1_000_000_000;
type Tags = Vec<(String, Value)>;

struct Workload {
    name: &'static str,
    namespace: &'static str,
    kind: &'static str,
    replicas: usize,
    cpu: f64,
    memory: f64,
}

const WORKLOADS: &[Workload] = &[
    Workload {
        name: "api-gateway",
        namespace: "storefront",
        kind: "deployment",
        replicas: 3,
        cpu: 0.5,
        memory: 0.5,
    },
    Workload {
        name: "storefront-api",
        namespace: "storefront",
        kind: "deployment",
        replicas: 3,
        cpu: 0.75,
        memory: 1.0,
    },
    Workload {
        name: "order-service",
        namespace: "checkout",
        kind: "deployment",
        replicas: 2,
        cpu: 0.5,
        memory: 0.75,
    },
    Workload {
        name: "inventory-service",
        namespace: "checkout",
        kind: "deployment",
        replicas: 2,
        cpu: 0.5,
        memory: 0.5,
    },
    Workload {
        name: "catalog-store",
        namespace: "storefront",
        kind: "statefulset",
        replicas: 2,
        cpu: 0.5,
        memory: 1.0,
    },
    Workload {
        name: "telemetry-collector",
        namespace: "observability",
        kind: "daemonset",
        replicas: 0,
        cpu: 0.1,
        memory: 0.125,
    },
    Workload {
        name: "inventory-reconcile",
        namespace: "checkout",
        kind: "job",
        replicas: 1,
        cpu: 0.25,
        memory: 0.25,
    },
];

fn tag(tags: &mut Tags, key: &str, value: impl Into<Value>) {
    tags.push((key.into(), value.into()));
}

fn resource(tags: &Tags, metrics: Vec<Value>) -> Value {
    json!({ "resource": { "attributes": attrs_json(tags) },
        "scopeMetrics": [{ "scope": { "name": "oc-tracegen/kubernetes" }, "metrics": metrics }] })
}

fn gauge(name: &str, unit: &str, value: f64, now: u64, tags: &Tags) -> Value {
    json!({ "name": name, "unit": unit, "description": format!("Synthetic Kubernetes demo: {name}"),
        "gauge": { "dataPoints": [{ "timeUnixNano": now.to_string(), "asDouble": value, "attributes": attrs_json(tags) }] } })
}

fn add(metrics: &mut Vec<Value>, now: u64, values: &[(&str, &str, f64)]) {
    metrics.extend(
        values
            .iter()
            .map(|(name, unit, value)| gauge(name, unit, *value, now, &vec![])),
    );
}

/// Stable identities with smooth time variation also allow bounded history to be regenerated.
pub fn metrics_payload(now: u64) -> Value {
    let mut resources = Vec::new();
    let wave = |phase: usize| 0.55 + 0.15 * ((now / SECOND) as f64 / 180.0 + phase as f64).sin();
    for (cluster, env) in [("shop-prod-eks", "prod"), ("shop-staging-eks", "staging")] {
        let nodes: Vec<_> = HOSTS
            .iter()
            .filter(|h| h.cluster == Some(cluster))
            .collect();
        let base = vec![
            ("k8s.cluster.name".into(), json!(cluster)),
            ("deployment.environment".into(), json!(env)),
            ("telemetry.synthetic".into(), json!(true)),
        ];
        let mut node_memory = vec![0.0; nodes.len()];
        for (wi, w) in WORKLOADS.iter().enumerate() {
            let desired = if w.kind == "daemonset" {
                nodes.len()
            } else if env == "staging" {
                1
            } else {
                w.replicas
            };
            // One staging pod periodically waits for capacity. Replicas, readiness and phases agree.
            let pending = usize::from(
                env == "staging"
                    && w.name == "inventory-service"
                    && (now / SECOND / 300).is_multiple_of(3),
            );
            let ready = desired - pending;
            let cpu_request = w.cpu * if env == "staging" { 0.35 } else { 1.0 };
            let mut tags = base.clone();
            tag(&mut tags, "k8s.namespace.name", w.namespace);
            tag(&mut tags, &format!("k8s.{}.name", w.kind), w.name);
            tag(&mut tags, "kube_service", w.name);
            if w.kind == "deployment" {
                tag(
                    &mut tags,
                    "k8s.replicaset.name",
                    format!("{}-7d9b6c", w.name),
                );
            }
            if w.kind == "job" {
                tag(&mut tags, "k8s.cronjob.name", "inventory-reconcile");
            }
            let mut metrics = Vec::new();
            add(
                &mut metrics,
                now,
                &[(
                    &format!("kubernetes_state.{}.count", w.kind),
                    "{object}",
                    1.0,
                )],
            );
            if w.kind == "job" {
                add(
                    &mut metrics,
                    now,
                    &[
                        ("kubernetes_state.cronjob.count", "{object}", 1.0),
                        ("kubernetes_state.job.succeeded", "{job}", 1.0),
                        ("kubernetes_state.job.failed", "{job}", 0.0),
                        ("kubernetes_state.job.duration", "s", 42.0),
                    ],
                );
            } else if w.kind == "daemonset" {
                for field in ["desired", "ready", "scheduled", "updated", "misscheduled"] {
                    add(
                        &mut metrics,
                        now,
                        &[(
                            &format!("kubernetes_state.daemonset.{field}"),
                            "{pod}",
                            if field == "misscheduled" {
                                0.0
                            } else {
                                desired as f64
                            },
                        )],
                    );
                }
            } else {
                for (field, count) in [
                    ("replicas", desired),
                    ("replicas_desired", desired),
                    ("replicas_available", ready),
                    ("replicas_ready", ready),
                    ("replicas_updated", ready),
                    ("replicas_current", desired),
                    ("replicas_unavailable", pending),
                ] {
                    add(
                        &mut metrics,
                        now,
                        &[(
                            &format!("kubernetes_state.{}.{field}", w.kind),
                            "{pod}",
                            count as f64,
                        )],
                    );
                }
                if w.kind == "deployment" {
                    add(
                        &mut metrics,
                        now,
                        &[
                            (
                                "kubernetes_state.deployment.rollingupdate.max_unavailable",
                                "{pod}",
                                1.0,
                            ),
                            (
                                "kubernetes_state.replicaset.replicas",
                                "{pod}",
                                desired as f64,
                            ),
                            (
                                "kubernetes_state.replicaset.replicas_desired",
                                "{pod}",
                                desired as f64,
                            ),
                            (
                                "kubernetes_state.replicaset.replicas_ready",
                                "{pod}",
                                ready as f64,
                            ),
                        ],
                    );
                }
            }
            if matches!(w.kind, "deployment" | "statefulset") {
                add(
                    &mut metrics,
                    now,
                    &[("kubernetes_state.service.count", "{service}", 1.0)],
                );
            }
            resources.push(resource(&tags, metrics));
            for replica in 0..desired {
                let ni = (wi + replica) % nodes.len();
                let host = nodes[ni];
                let running = replica >= pending && w.kind != "job";
                let phase = if w.kind == "job" {
                    "succeeded"
                } else if running {
                    "running"
                } else {
                    "pending"
                };
                let cpu = if running {
                    cpu_request * wave(wi + replica)
                } else {
                    0.0
                };
                let memory = if running {
                    w.memory * GIB * (0.7 + wave(wi) * 0.1)
                } else {
                    0.0
                };
                node_memory[ni] += memory;
                let mut pt = tags.clone();
                tag(&mut pt, "host.name", host.name);
                tag(&mut pt, "k8s.node.name", host.name);
                tag(&mut pt, "k8s.pod.name", format!("{}-{replica:03}", w.name));
                tag(&mut pt, "k8s.container.name", w.name);
                tag(&mut pt, "service.name", w.name);
                tag(&mut pt, "pod_phase", phase);
                let mut m = Vec::new();
                add(
                    &mut m,
                    now,
                    &[
                        ("kubernetes.pods.running", "{pod}", f64::from(running)),
                        (
                            "kubernetes.containers.running",
                            "{container}",
                            f64::from(running),
                        ),
                        (
                            "kubernetes_state.container.running",
                            "{container}",
                            f64::from(running),
                        ),
                        (
                            "kubernetes_state.container.ready",
                            "{container}",
                            f64::from(running),
                        ),
                        (
                            "kubernetes_state.container.terminated",
                            "{container}",
                            f64::from(w.kind == "job"),
                        ),
                        ("kubernetes.cpu.requests", "{core}", cpu_request),
                        ("kubernetes.cpu.limits", "{core}", cpu_request * 2.0),
                        (
                            "kubernetes_state.container.cpu_requested",
                            "{core}",
                            cpu_request,
                        ),
                        (
                            "kubernetes.cpu.usage.total",
                            "n{core}",
                            cpu * 1_000_000_000.0,
                        ),
                        ("container.cpu.usage", "{core}", cpu),
                        ("kubernetes.memory.requests", "By", w.memory * GIB),
                        ("kubernetes.memory.limits", "By", w.memory * GIB * 2.0),
                        ("kubernetes.memory.usage", "By", memory),
                        ("container.memory.usage", "By", memory),
                        ("container.memory.working_set", "By", memory * 0.9),
                        ("kubernetes.memory.rss", "By", memory * 0.85),
                        ("kubernetes.filesystem.usage", "By", 0.4 * GIB),
                        ("kubernetes.filesystem.usage_pct", "1", 0.04),
                        ("kubernetes.containers.restarts", "{restart}", 0.0),
                        ("kubernetes_state.container.restarts", "{restart}", 0.0),
                    ],
                );
                for (name, unit, rate) in [
                    ("kubernetes.network.rx_bytes", "By/s", 80_000.0),
                    ("kubernetes.network.tx_bytes", "By/s", 50_000.0),
                    ("kubernetes.io.read_bytes", "By/s", 16_000.0),
                    ("kubernetes.io.write_bytes", "By/s", 8_000.0),
                    ("kubernetes.network.rx_errors", "{error}/s", 0.0),
                    ("kubernetes.network.tx_errors", "{error}/s", 0.0),
                    ("kubernetes.network.rx_dropped", "{packet}/s", 0.0),
                    ("kubernetes.network.tx_dropped", "{packet}/s", 0.0),
                    ("kubernetes.network_errors", "{error}/s", 0.0),
                ] {
                    add(
                        &mut m,
                        now,
                        &[(
                            name,
                            unit,
                            if running {
                                rate * wave(wi + replica)
                            } else {
                                0.0
                            },
                        )],
                    );
                }
                for condition in ["true", "false"] {
                    m.push(gauge(
                        "kubernetes_state.pod.ready",
                        "{pod}",
                        f64::from((condition == "true") == running),
                        now,
                        &vec![("condition".into(), json!(condition))],
                    ));
                }
                add(
                    &mut m,
                    now,
                    &[("kubernetes_state.pod.status_phase", "{pod}", 1.0)],
                );
                m.push(gauge(
                    "kubernetes_state.container.status_report.count.waiting",
                    "{container}",
                    pending as f64 * f64::from(replica == 0),
                    now,
                    &vec![("reason".into(), json!("containercreating"))],
                ));
                m.push(gauge(
                    "kubernetes.containers.last_state.terminated",
                    "{container}",
                    0.0,
                    now,
                    &vec![("reason".into(), json!("oomkilled"))],
                ));
                resources.push(resource(&pt, m));
            }
        }
        resources.push(control_plane(&base, now, env));
        for (i, host) in nodes.iter().enumerate() {
            let mut tags = base.clone();
            tag(&mut tags, "host.name", host.name);
            tag(&mut tags, "k8s.node.name", host.name);
            let mut m = Vec::new();
            add(
                &mut m,
                now,
                &[
                    ("kubernetes_state.node.count", "{node}", 1.0),
                    (
                        "kubernetes_state.node.cpu_capacity",
                        "{core}",
                        host.cpus as f64,
                    ),
                    (
                        "kubernetes_state.node.cpu_allocatable",
                        "{core}",
                        host.cpus as f64 - 0.5,
                    ),
                    (
                        "kubernetes_state.node.memory_capacity",
                        "By",
                        host.mem_gib * GIB,
                    ),
                    (
                        "kubernetes_state.node.memory_allocatable",
                        "By",
                        (host.mem_gib - 1.0) * GIB,
                    ),
                    ("kubernetes_state.node.pods_capacity", "{pod}", 110.0),
                    ("kubernetes_state.node.pods_allocatable", "{pod}", 110.0),
                    (
                        "kubernetes_state.node.ephemeral_storage_capacity",
                        "By",
                        100.0 * GIB,
                    ),
                    ("kubernetes.node.filesystem.usage", "By", 35.0 * GIB),
                    ("kubernetes.node.filesystem.usage_pct", "1", 0.35),
                    (
                        "kubernetes.memory.usage_pct",
                        "1",
                        node_memory[i] / (host.mem_gib * GIB),
                    ),
                    ("tls.seconds_left", "s", 90.0 * 86400.0),
                ],
            );
            for condition in [
                "ready",
                "memorypressure",
                "diskpressure",
                "pidpressure",
                "networkunavailable",
            ] {
                for status in ["true", "false"] {
                    let value = f64::from((condition == "ready") == (status == "true"));
                    let attrs = vec![
                        ("condition".into(), json!(condition)),
                        ("status".into(), json!(status)),
                    ];
                    for name in [
                        "kubernetes_state.node.by_condition",
                        "kubernetes_state.nodes.by_condition",
                    ] {
                        m.push(gauge(name, "{node}", value, now, &attrs));
                    }
                }
            }
            add(
                &mut m,
                now,
                &[
                    ("system.cpu.num_cores", "{core}", host.cpus as f64),
                    ("system.mem.total", "GiBy", host.mem_gib),
                    (
                        "system.mem.free",
                        "GiBy",
                        host.mem_gib - node_memory[i] / GIB - 1.0,
                    ),
                    (
                        "system.mem.pct_usable",
                        "1",
                        1.0 - (node_memory[i] / GIB + 1.0) / host.mem_gib,
                    ),
                ],
            );
            for check in ["kubernetes.kubelet.check", "kubernetes.kubelet.check.ping"] {
                m.push(gauge(
                    "service_check.status",
                    "1",
                    0.0,
                    now,
                    &vec![("check".into(), json!(check))],
                ));
            }
            resources.push(resource(&tags, m));
        }
    }
    json!({ "resourceMetrics": resources })
}

/// Model a small healthy control plane. Cumulative counters grow from a stable boot timestamp;
/// paired duration sums/counts describe a consistent latency rather than unrelated random values.
fn control_plane(base: &Tags, now: u64, env: &str) -> Value {
    let mut tags = base.clone();
    for (key, value) in [
        (
            "cluster_name",
            if env == "prod" {
                "shop-prod-eks"
            } else {
                "shop-staging-eks"
            },
        ),
        ("verb", "GET"),
        ("method", "GET"),
        ("resource", "pods"),
        ("code", "200"),
        ("grpc_code", "OK"),
        ("grpc_service", "etcdserverpb.KV"),
        ("queue", "active"),
        ("event", "PodUpdate"),
        ("attempts", "1"),
        ("result", "scheduled"),
        ("storage_cluster_id", "demo-etcd"),
    ] {
        tag(&mut tags, key, value);
    }
    let mut m = Vec::new();
    let factor = if env == "prod" { 1.0 } else { 0.2 };
    // Fixed fictional boot epoch preserves counter identity over generator restarts/backfill.
    let boot = 1_700_000_000 * SECOND;
    let elapsed = now.saturating_sub(boot) as f64 / SECOND as f64;
    for (name, rate, unit) in [
        ("kube_apiserver.apiserver_request_total", 30.0, "{request}"),
        (
            "kube_apiserver.apiserver_request_total.count",
            30.0,
            "{request}",
        ),
        ("kube_apiserver.apiserver_request.count", 30.0, "{request}"),
        (
            "kube_apiserver.request_duration_seconds.count",
            30.0,
            "{request}",
        ),
        ("kube_apiserver.request_duration_seconds.sum", 0.45, "s"),
        (
            "kube_apiserver.apiserver_request_duration_seconds.count",
            30.0,
            "{request}",
        ),
        (
            "kube_apiserver.apiserver_request_duration_seconds.sum",
            0.45,
            "s",
        ),
        ("kube_apiserver.etcd_requests_total", 45.0, "{request}"),
        ("kube_apiserver.etcd_request_errors_total", 0.0, "{error}"),
        (
            "kube_apiserver.grpc_client_handled_total",
            45.0,
            "{request}",
        ),
        (
            "kube_apiserver.rest_client_requests_total.count",
            12.0,
            "{request}",
        ),
        (
            "etcd.disk.backend.commit.duration.seconds.count",
            8.0,
            "{commit}",
        ),
        ("etcd.disk.backend.commit.duration.seconds.sum", 0.024, "s"),
        ("etcd.grpc.server.msg.sent.total", 45.0, "{message}"),
        ("etcd.grpc.server.msg.received.total", 45.0, "{message}"),
        (
            "etcd.debugging.mvcc.db.compaction.total.duration.milliseconds.count",
            0.1,
            "{compaction}",
        ),
        (
            "etcd.debugging.mvcc.db.compaction.total.duration.milliseconds.sum",
            0.8,
            "ms",
        ),
        (
            "etcd.debugging.mvcc.db.compaction.pause.duration.milliseconds.count",
            0.1,
            "{compaction}",
        ),
        (
            "etcd.debugging.mvcc.db.compaction.pause.duration.milliseconds.sum",
            0.05,
            "ms",
        ),
        ("etcd.debugging.mvcc.db.compaction.keys.total", 0.4, "{key}"),
        ("etcd.debugging.store.writes.total", 8.0, "{operation}"),
        ("etcd.debugging.store.reads.total", 37.0, "{operation}"),
        ("etcd.disk.wal.write.bytes.total", 4096.0, "By"),
        ("etcd.debugging.mvcc.put.total", 7.8, "{operation}"),
        ("etcd.debugging.mvcc.range.total", 37.0, "{operation}"),
        ("etcd.debugging.mvcc.events.total", 8.0, "{event}"),
        ("etcd.debugging.mvcc.delete.total", 0.2, "{operation}"),
        ("kube_scheduler.queue.incoming_pods", 0.1, "{pod}"),
        (
            "kube_scheduler.scheduling.pod.scheduling_duration.count",
            0.1,
            "{pod}",
        ),
        (
            "kube_scheduler.scheduling.pod.scheduling_duration.sum",
            0.002,
            "s",
        ),
        ("kube_scheduler.schedule_attempts", 0.1, "{attempt}"),
        ("kube_scheduler.client.http.requests", 2.0, "{request}"),
        ("kube_controller_manager.nodes.evictions", 0.0, "{node}"),
        ("kube_controller_manager.queue.adds", 0.5, "{item}"),
        (
            "kube_controller_manager.client.http.requests",
            5.0,
            "{request}",
        ),
        (
            "kube_controller_manager.queue.queue_duration.count",
            0.5,
            "{item}",
        ),
        (
            "kube_controller_manager.queue.queue_duration.sum",
            0.05,
            "s",
        ),
    ] {
        m.push(json!({ "name": name, "unit": unit, "description": "Synthetic Kubernetes control plane",
            "sum": { "aggregationTemporality": 2, "isMonotonic": true, "dataPoints": [{
                "startTimeUnixNano": boot.to_string(), "timeUnixNano": now.to_string(), "asDouble": elapsed * rate * factor
            }] } }));
    }
    add(
        &mut m,
        now,
        &[
            ("kube_apiserver.current_inflight_requests", "{request}", 3.0),
            (
                "kube_apiserver.etcd.db.total_size",
                "By",
                64.0 * 1024.0 * 1024.0,
            ),
            ("etcd.debugging.mvcc.keys.total", "{key}", 2400.0),
            ("etcd.process.open.fds", "{file}", 64.0),
            ("etcd.process.max.fds", "{file}", 65536.0),
            (
                "kube_scheduler.pending_pods",
                "{pod}",
                f64::from(env == "staging" && (now / SECOND / 300).is_multiple_of(3)),
            ),
            ("kube_controller_manager.nodes.unhealthy", "{node}", 0.0),
            ("kube_controller_manager.queue.depth", "{item}", 2.0),
        ],
    );
    for check in [
        "kube_apiserver_controlplane.up",
        "etcd.prometheus.health",
        "kube_scheduler.up",
        "kube_controller_manager.up",
    ] {
        for status in ["ok", "critical", "unknown"] {
            add(
                &mut m,
                now,
                &[(
                    &format!("check_run.{check}.{status}"),
                    "{check}",
                    f64::from(status == "ok"),
                )],
            );
        }
    }
    // Explicit zero series make healthy HTTP error panels show zero rather than missing telemetry.
    for code in ["404", "429", "500"] {
        for name in [
            "kube_apiserver.apiserver_request_total.count",
            "kube_apiserver.apiserver_request.count",
        ] {
            m.push(json!({ "name": name, "unit": "{request}", "sum": { "aggregationTemporality": 2, "isMonotonic": true,
                "dataPoints": [{ "startTimeUnixNano": boot.to_string(), "timeUnixNano": now.to_string(), "asDouble": 0.0,
                    "attributes": attrs_json(&[("code".into(), json!(code))]) }] } }));
        }
    }
    add(
        &mut m,
        now,
        &[("kubernetes_state.crd.count", "{object}", 0.0)],
    );
    resource(&tags, m)
}

/// Kubernetes lifecycle events go through the authenticated events intake.
pub async fn send_events(http: &reqwest::Client, url: &str, now: u64) {
    for (env, cluster) in [("prod", "shop-prod-eks"), ("staging", "shop-staging-eks")] {
        let warning = env == "staging" && (now / SECOND / 300).is_multiple_of(3);
        let event = json!({
            "title": if warning { "Pod scheduling delayed" } else { "Workload reconciliation complete" },
            "text": if warning { "The inventory-reconcile job is waiting for available scheduling capacity." }
                else { "The storefront-api deployment has reached its desired replica count." },
            "date_happened": now / SECOND, "source_type_name": "kubernetes",
            "alert_type": if warning { "warning" } else { "info" },
            "aggregation_key": format!("{cluster}-workload-reconciliation"),
            "tags": [format!("env:{env}"), format!("kube_cluster_name:{cluster}"),
                format!("kube_namespace:{}", if warning { "checkout" } else { "storefront" }),
                format!("{}:{}", if warning { "kube_job" } else { "kube_deployment" },
                    if warning { "inventory-reconcile" } else { "storefront-api" })]
        });
        if let Err(error) = http
            .post(url)
            .json(&event)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            tracing::warn!(%error, "Kubernetes event export failed");
        }
    }
}

/// Seed a half-hour at one-minute resolution, bounded and through intake (never direct SQL inserts).
pub async fn backfill(http: &reqwest::Client, url: &str, now: u64) {
    let minutes = std::env::var("OC_K8S_BACKFILL_MINUTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30)
        .min(60);
    for minutes in (1..=minutes).rev() {
        let payload = metrics_payload(now.saturating_sub(minutes * 60 * SECOND));
        if let Err(e) = http
            .post(url)
            .json(&payload)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            tracing::warn!(error = %e, "Kubernetes history backfill stopped");
            return;
        }
    }
    tracing::info!(minutes, "Kubernetes demo history exported");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn bundled_dashboard_metric_names_are_covered() {
        use std::collections::BTreeSet;
        let payload = metrics_payload(1_800_000_000 * SECOND);
        let mut names = BTreeSet::new();
        for resource in payload["resourceMetrics"].as_array().unwrap() {
            for metric in resource["scopeMetrics"][0]["metrics"].as_array().unwrap() {
                names.insert(metric["name"].as_str().unwrap().to_string());
            }
        }
        // Host metrics are emitted by hosts.rs; these aliases are resolved by the API.
        names.extend(
            [
                "system.cpu.idle",
                "system.cpu.user",
                "system.cpu.system",
                "system.mem.used",
            ]
            .map(str::to_string),
        );
        let dashboards: Value = serde_json::from_str(include_str!(
            "../../../seeds/integrations/kubernetes/dashboards.json"
        ))
        .unwrap();
        fn check(value: &Value, names: &BTreeSet<String>) {
            match value {
                Value::Object(fields) => fields.values().for_each(|v| check(v, names)),
                Value::Array(items) => items.iter().for_each(|v| check(v, names)),
                Value::String(s)
                    if ["avg:", "sum:", "min:", "max:"]
                        .iter()
                        .any(|prefix| s.starts_with(prefix)) =>
                {
                    let metric = s.split_once(':').unwrap().1.split('{').next().unwrap();
                    assert!(
                        names.contains(metric),
                        "dashboard requires missing demo metric {metric}"
                    );
                }
                _ => {}
            }
        }
        check(&dashboards, &names);
    }

    #[test]
    fn control_plane_counters_are_monotonic_and_latency_pairs_agree() {
        let a = control_plane(&vec![], 1_800_000_000 * SECOND, "prod");
        let b = control_plane(&vec![], (1_800_000_000 + 60) * SECOND, "prod");
        let mut rates = BTreeMap::new();
        for (a, b) in a["scopeMetrics"][0]["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .zip(b["scopeMetrics"][0]["metrics"].as_array().unwrap())
        {
            if a.get("sum").is_none() {
                continue;
            }
            assert_eq!(a["sum"]["aggregationTemporality"], 2);
            assert_eq!(a["sum"]["isMonotonic"], true);
            let av = a["sum"]["dataPoints"][0]["asDouble"].as_f64().unwrap();
            let bv = b["sum"]["dataPoints"][0]["asDouble"].as_f64().unwrap();
            assert!(bv >= av);
            if a["sum"]["dataPoints"][0].get("attributes").is_none() {
                rates.insert(a["name"].as_str().unwrap(), (bv - av) / 60.0);
            }
        }
        assert!(
            (rates["kube_apiserver.request_duration_seconds.sum"]
                / rates["kube_apiserver.request_duration_seconds.count"]
                - 0.015)
                .abs()
                < 1e-8
        );
    }

    #[test]
    fn replicas_resources_and_pod_states_agree() {
        let payload = metrics_payload(1_800 * SECOND);
        let mut totals = BTreeMap::<String, f64>::new();
        let mut pods = 0;
        let mut nodes = 0;
        for r in payload["resourceMetrics"].as_array().unwrap() {
            let attrs = r["resource"]["attributes"].as_array().unwrap();
            assert!(attrs
                .iter()
                .any(|a| a["key"] == "telemetry.synthetic" && a["value"]["boolValue"] == true));
            let metrics = r["scopeMetrics"][0]["metrics"].as_array().unwrap();
            let get = |name: &str| {
                metrics
                    .iter()
                    .find(|m| m["name"] == name)
                    .map(|m| m["gauge"]["dataPoints"][0]["asDouble"].as_f64().unwrap())
            };
            if let Some(cpu) = get("kubernetes.cpu.usage.total") {
                pods += 1;
                assert!(cpu <= get("kubernetes.cpu.requests").unwrap() * 1_000_000_000.0);
                assert!(
                    get("kubernetes.memory.usage").unwrap()
                        <= get("kubernetes.memory.requests").unwrap()
                );
                let ready: f64 = metrics
                    .iter()
                    .filter(|m| m["name"] == "kubernetes_state.pod.ready")
                    .map(|m| m["gauge"]["dataPoints"][0]["asDouble"].as_f64().unwrap())
                    .sum();
                assert_eq!(ready, 1.0);
            }
            nodes += usize::from(get("kubernetes_state.node.count").is_some());
            for m in metrics {
                let name = m["name"].as_str().unwrap();
                let data = m.get("gauge").or_else(|| m.get("sum")).unwrap();
                let v = data["dataPoints"][0]["asDouble"].as_f64().unwrap();
                assert!(v.is_finite() && v >= 0.0, "{name}");
                *totals.entry(name.into()).or_default() += v;
            }
        }
        assert_eq!(nodes, 4);
        assert_eq!(pods, 23);
        assert_eq!(
            totals["kubernetes_state.deployment.replicas_desired"],
            totals["kubernetes_state.deployment.replicas_available"]
                + totals["kubernetes_state.deployment.replicas_unavailable"]
        );
        assert_eq!(
            totals["kubernetes.pods.running"],
            totals["kubernetes_state.container.running"]
        );
    }
}
