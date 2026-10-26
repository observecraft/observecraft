# ObserveCraft Agent

Collects logs, metrics, traces and Kubernetes state from a cluster and sends them to
[ObserveCraft](https://github.com/observecraft/observecraft), the open-source, self-hosted observability platform
(chart: [`observecraft`](https://github.com/observecraft/observecraft/tree/main/deploy/helm/observecraft)).

The agent is the official **OpenTelemetry Collector** (`otel/opentelemetry-collector-contrib`), configured by this chart.
Use it for clusters that don't run the Datadog Agent. If you already run the Datadog Agent (Helm chart or Operator),
point it at ObserveCraft instead and skip this chart: see
[`deploy/examples/datadog`](https://github.com/observecraft/observecraft/tree/main/deploy/examples/datadog).

| Workload | What it collects |
|---|---|
| **Node agent** (DaemonSet) | Container logs; kubelet pod and container metrics; host metrics; pods annotated `prometheus.io/scrape: "true"`; and, from applications on the node, OTLP (`:4317` gRPC, `:4318` HTTP), dd-trace traces (`:8126`) and DogStatsD (`:8125/udp`) |
| **Cluster agent** (Deployment) | Cluster metrics, Kubernetes object snapshots for the Kubernetes Explorer, Kubernetes events |

Tags follow Datadog's names (`kube_namespace`, `pod_name`, `kube_deployment`, `kube_cluster_name`, …), so ObserveCraft's
Kubernetes pages, dashboards and monitors work the same as with the Datadog Agent. For unified service tagging, set the
pod labels `tags.datadoghq.com/service`, `tags.datadoghq.com/env` and `tags.datadoghq.com/version` (or
`OTEL_SERVICE_NAME`).

## Install

Create an API key in ObserveCraft (*Organization Settings → API Keys*), then:

```bash
kubectl create namespace observecraft
kubectl -n observecraft create secret generic observecraft-agent --from-literal api-key=ocik_…

helm install observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent -n observecraft \
  --set endpoint=https://intake.observecraft.example.com \
  --set existingSecret=observecraft-agent \
  --set clusterName=prod-eu
```

In the same cluster as ObserveCraft, use the in-cluster intake: `--set endpoint=http://observecraft-intake.observecraft.svc:4318`.

## Send from your applications

With `node.hostPorts: true` (the default), applications reach the agent on their node's IP:

```yaml
env:
  - name: HOST_IP
    valueFrom: { fieldRef: { fieldPath: status.hostIP } }
  # OpenTelemetry SDKs
  - name: OTEL_EXPORTER_OTLP_ENDPOINT
    value: http://$(HOST_IP):4318
  # Datadog tracing libraries and DogStatsD clients
  - name: DD_AGENT_HOST
    value: $(HOST_IP)
```

The agent is also reachable through the node-local service `<release>-observecraft-agent.<namespace>.svc`.

## Main settings

| Key | Default | Description |
|---|---|---|
| `endpoint` | `""` | ObserveCraft intake URL. Required. |
| `existingSecret` / `apiKey` | `""` | Secret holding the API key under `api-key` (preferred), or the key itself. |
| `clusterName` | `""` | Sent as `kube_cluster_name` on everything from this cluster. |
| `node.enabled` / `cluster.enabled` | `true` | Run the node agent / the cluster agent. |
| `node.logs.enabled`, `node.logs.excludeNamespaces` | `true`, `[]` | Container logs, and namespaces to skip (the agent's own are always skipped). |
| `node.kubeletMetrics.*`, `node.hostMetrics.*` | enabled, `20s` | Kubelet and host metrics and their interval. |
| `node.otlp.enabled`, `node.datadog.traces`, `node.datadog.dogstatsd` | `true` | Application endpoints on the node. |
| `node.hostPorts` | `true` | Expose the application endpoints on each node's IP. |
| `node.prometheus.*` | enabled, `30s` | Scrape annotated pods. |
| `node.dataHostPath` | `/var/lib/observecraft-agent` | Send queue and log read positions, kept across agent restarts. |
| `cluster.objectsInterval`, `cluster.events` | `60s`, `true` | Object snapshots for the Kubernetes Explorer, and events. |
| `node.extraConfig` / `cluster.extraConfig` | `{}` | Merged over the generated Collector configuration (extra receivers, processors, …). |
| `image.repository` / `image.tag` | `otel/opentelemetry-collector-contrib` / `0.112.0` | Collector image. |

Every value is documented in [`values.yaml`](https://github.com/observecraft/observecraft/blob/main/deploy/helm/observecraft-agent/values.yaml)
and validated by `values.schema.json`.

## Upgrade and uninstall

```bash
helm upgrade observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent --version <new version> -n observecraft --reuse-values
helm uninstall observecraft-agent -n observecraft
```

Releases are signed with Sigstore cosign; see
[Verifying releases](https://github.com/observecraft/observecraft/blob/main/SECURITY.md#verifying-releases).
