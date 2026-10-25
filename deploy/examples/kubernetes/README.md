# Kubernetes → ObserveCraft

`observecraft-collector.yaml` runs two OpenTelemetry Collectors (contrib 0.112), the equivalent of the
Datadog Agent and Cluster Agent:

| Collector | Kind | Collects |
|---|---|---|
| `observecraft-agent` | DaemonSet | container logs (`/var/log/pods`), Kubernetes metadata on every record, kubelet CPU/memory per node, pod and container, node host metrics |
| `observecraft-cluster` | Deployment (1) | cluster state metrics (phases, restarts, requests/limits, replicas, node conditions), every Kubernetes object each minute, Kubernetes events |

ObserveCraft turns the metadata into Datadog's tags (`kube_cluster_name`, `kube_namespace`, `pod_name`,
`kube_deployment`, `kube_container_name`, `kube_node`, `image_name`, `short_image`, …). JSON log lines are
parsed (`level`/`severity`/`status`, `msg`/`message`/`log`, timestamps, `trace_id`), and so are
plain-text levels (`ERROR …`, klog `E1025 …`).

**Service, env and version** follow Datadog's unified service tagging: the pod labels
`tags.datadoghq.com/service|env|version`, then `OTEL_SERVICE_NAME` / `OTEL_RESOURCE_ATTRIBUTES` (via the
OTel SDK), then the workload name (deployment, statefulset, …), then the container name.

## Install

```bash
kubectl create namespace observecraft
kubectl -n observecraft create secret generic observecraft \
  --from-literal=api-key=ocik_…                    `# Organization Settings → API Keys` \
  --from-literal=endpoint=https://intake.example.com \
  --from-literal=cluster-name=prod-eu
kubectl apply -f observecraft-collector.yaml
```

Logs show up in the Log Explorer (`kube_cluster_name:prod-eu`), metrics as `k8s.*` and `container.*`,
and objects and events in the Kubernetes Explorer.

## Try it locally

With ObserveCraft running on this machine (`make up`, `make server`, `make intake`) and a local cluster
(kind, OrbStack, Docker Desktop, minikube):

```bash
KUBE_CONTEXT=kind-dev OBSERVECRAFT_API_KEY=ocik_… CLUSTER_NAME=local-dev ./local-test.sh
KUBE_CONTEXT=kind-dev ./local-test.sh delete
```

It installs the collectors plus `demo-workloads.yaml` (a `shop` namespace with JSON and plain-text
loggers, a CrashLoopBackOff pod, a DaemonSet and a CronJob). The script only accepts local contexts.
