# Sending data to ObserveCraft

Everything goes to the **intake**, authenticated with an **ingest API key** (*Organization Settings → API Keys*,
`ocik_…`). The key decides the organization the data belongs to.

The examples use `https://intake.observecraft.example.com` (the Helm chart's `ingress.intake.host`; `http://localhost:4318`
with Docker Compose) and `ocik_…` for the key. Keep the key in a secret, not in config files.

## What the intake accepts

| Protocol | Endpoint | Signals | Key header |
|---|---|---|---|
| OTLP/HTTP (protobuf or JSON, gzip) | `POST /v1/traces`, `/v1/metrics`, `/v1/logs` (port 4318) | traces, metrics, logs | `Authorization: Bearer ocik_…` |
| OTLP/gRPC (gzip, zstd) | port 4317 (Helm: `ingress.intakeGrpc`) | traces, metrics, logs | `authorization: Bearer ocik_…` metadata |
| Datadog logs API | `POST /api/v2/logs` (JSON, gzip / deflate / zstd) | logs | `DD-API-KEY: ocik_…` |
| Datadog metrics API | `POST /api/v2/series` (JSON or the Agent's protobuf), `POST /api/v1/series` (JSON), `POST /api/beta/sketches` (distributions) | metrics | `DD-API-KEY: ocik_…` |
| Datadog Agent APM | `POST /api/v0.2/traces` (the trace-agent's payloads, both formats), `/api/v0.2/stats` | traces | `DD-API-KEY: ocik_…` |
| Datadog Agent service checks, Kubernetes | `POST /api/v1/check_run`, `POST /api/v2/orchmanif` (Cluster Agent manifests) | service checks, Kubernetes objects | `DD-API-KEY: ocik_…` |

Any of the three headers works on every endpoint: `Authorization: Bearer`, `OC-API-Key` or `DD-API-KEY`. As with
Datadog, the query parameters `?api_key=` and `?dd-api-key=` work too.

These are the protocols Datadog accepts. As with Datadog, there is no Elasticsearch, Splunk HEC or Prometheus
remote-write endpoint: those senders go through an agent that speaks one of the protocols above (see below).

Datadog data is stored under its own names, so existing Datadog dashboards, monitors and queries keep working.

- **Metric types:** `gauge` and `rate` become gauges, `count` a delta monotonic sum, and distributions (DDSketch) delta
  exponential histograms with percentiles.
- **Service checks** become the gauge `service_check.status` (0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN), tagged
  `check:<name>`.
- **Tags:**
  - the `host` becomes `host.name`, and the `service:` tag becomes `service.name`;
  - container and Kubernetes tags (`kube_namespace`, `pod_name`, `kube_deployment`, `container_id`, `env`, `version`,
    …) become the matching OpenTelemetry resource attributes, so the Kubernetes and host pages read them;
  - queries still use the Datadog names;
  - other `key:value` tags become attributes.
- **Spans** keep Datadog's resource as the span name. The operation name goes to `operation.name` and the span type
  to `span.type`.

## Choosing a sender

| You have | Use |
|---|---|
| Kubernetes | the **Datadog Agent** with Datadog's Helm chart or Operator; OpenTelemetry-only: the **observecraft-agent** chart (the official OpenTelemetry Collector as node and cluster agent) |
| Hosts / VMs | the **Datadog Agent**; OpenTelemetry-only: the OpenTelemetry Collector with `deploy/examples/otel-host/collector.yaml` |
| Applications | Datadog tracing libraries (dd-trace-*) and DogStatsD clients, unchanged, through the Datadog Agent; or OpenTelemetry SDKs (to a collector, or straight to the intake) |
| Fluent Bit, Fluentd, Vector, Logstash | their Datadog output (logs) |
| Filebeat | Filebeat → Vector (Beats protocol) → intake, or replace it with the Datadog Agent's log collection |
| Prometheus exporters | the Datadog Agent's OpenMetrics autodiscovery, or the OpenTelemetry Collector's (`prometheus.io/scrape: "true"`) |
| Syslog | the Datadog Agent's syslog log collection, or the OpenTelemetry Collector's `syslog` receiver |
| ECS / Fargate | FireLens (Fluent Bit) and ADOT, see `deploy/examples/ecs` |

## Datadog Agent, Helm chart and Operator

The Datadog Agent and Cluster Agent work unchanged. You change only where they send data and the key. Applications
keep Datadog's tracing libraries, profilers and DogStatsD clients as they are. What arrives, and where it shows:

| Data | Where |
|---|---|
| Metrics (including the checks from `integrations-core`), distributions | metrics, dashboards, monitors |
| Logs | log explorer |
| APM traces | APM |
| Continuous profiles | APM → Profiles (Explorer, Comparison, Insights) |
| Service checks | the `service_check.status` metric |
| Events (Agent, DogStatsD, events API) | Event Management → All Events |
| Host metadata (OS, hardware, agent version, cloud, host tags) | host list and host page |
| Live processes and containers | Processes explorer, the host page's and the Kubernetes side panel's Processes tabs; `process.*` and `container.*` metrics |
| Network connections (system-probe) | Cloud Network Monitoring (Analytics, Map) |
| Cluster Agent manifests, kubelet metrics | Kubernetes Explorer (tables, YAML, CPU and memory) |

**Kubernetes, with Datadog's Helm chart:** [`deploy/examples/datadog/values.yaml`](../deploy/examples/datadog/values.yaml)

```bash
kubectl -n datadog create secret generic datadog-secret --from-literal api-key=ocik_…
helm install datadog datadog/datadog -n datadog -f values.yaml --set datadog.clusterName=prod-eu
```

**Kubernetes, with the Datadog Operator:** [`deploy/examples/datadog/datadog-agent.yaml`](../deploy/examples/datadog/datadog-agent.yaml)
(a `DatadogAgent` resource).

**Hosts** (`datadog.yaml`):

```yaml
api_key: ocik_…
dd_url: https://intake.observecraft.example.com
apm_config:
  apm_dd_url: https://intake.observecraft.example.com
  profiling_dd_url: https://intake.observecraft.example.com/api/v2/profile
  telemetry: { enabled: false }
logs_enabled: true
logs_config:
  logs_dd_url: intake.observecraft.example.com:443
  force_use_http: true
  # logs_no_ssl: true   # only for a plain-HTTP intake
process_config:
  process_dd_url: https://intake.observecraft.example.com
  process_collection: { enabled: true }
# Network monitoring (needs system-probe): network_config: { enabled: true }
# Not a Datadog site: anything left unconfigured has nowhere to send.
site: observecraft.invalid
remote_configuration: { enabled: false }
```

### Continuous Profiler

Turn profiling on in the Datadog tracing library as usual (for example `DD_PROFILING_ENABLED=true`). The library
uploads a profile every minute to the Agent, and the Agent forwards it to `profiling_dd_url` (set in the examples
above). Profiles are kept for 8 days.

In APM → Profiles:
- **Explorer**: CPU (or any other sample type) over time per service, each service's share, and a service's flame
  graph merged over the time range, with its endpoints and its functions grouped by package. Profile List shows the
  individual uploads.
- **Comparison**: the same service over two time ranges (or two queries), flame graphs side by side with the
  differences coloured.
- **Insights**: patterns found in the profiles, such as CPU bursts, with a review status.

The Thread Timeline and Call Graph views aren't available yet. Go services send separate CPU and heap files; their
sample types (`cpu`, `inuse_space`, …) are shown under Datadog's names (CPU Time, Heap Live Size, …).

**What isn't supported yet:**
- Some payloads are acknowledged so the Agent doesn't keep retrying them, but they aren't stored: the Agent's
  inventories (`/api/v1/metadata`) and the orchestrator's summaries (`/api/v2/orch`).
- Cloud Network Monitoring resolves IPs to tags from the Kubernetes inventory, the hosts' Datadog metadata and the
  other Agents' reports; DNS names (`domain`), network paths and the DNS page aren't available yet.
- Not available yet: Cloud Security, the live debugger, data streams and remote configuration.
- The examples set `site` to a domain that isn't Datadog's, `observecraft.invalid`. They also turn off products that
  would send to Datadog's own intakes, so nothing the Agent collects goes to Datadog.
- Not available yet: APM telemetry and the security products.
- Run either the Datadog Agent or the `observecraft-agent` chart's node agent on a node, not both. Both listen on
  8125 and 8126. If you need both, set `node.hostPorts=false` on `observecraft-agent`.

## OpenTelemetry

SDKs (directly to the intake, or to a collector without the key):

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=https://intake.observecraft.example.com
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_HEADERS="authorization=Bearer ocik_…"
export OTEL_SERVICE_NAME=order-service
export OTEL_RESOURCE_ATTRIBUTES=deployment.environment=prod,service.version=1.4.2
```

Collector:

```yaml
exporters:
  otlphttp/observecraft:
    endpoint: https://intake.observecraft.example.com
    headers: { authorization: "Bearer ${env:OBSERVECRAFT_API_KEY}" }
    compression: gzip
  # or gRPC
  otlp/observecraft:
    endpoint: otlp.observecraft.example.com:443
    headers: { authorization: "Bearer ${env:OBSERVECRAFT_API_KEY}" }
```

## OpenTelemetry Collector (`observecraft-agent` chart)

For clusters and hosts that don't run the Datadog Agent. The chart runs the official
`otel/opentelemetry-collector-contrib` image, configured as a node agent and a cluster agent. It accepts OTLP, and
also Datadog tracers (8126) and DogStatsD (8125).

**Kubernetes:**

```bash
kubectl create namespace observecraft
kubectl -n observecraft create secret generic observecraft-agent --from-literal=api-key=ocik_…
helm install observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent -n observecraft \
  --set endpoint=https://intake.observecraft.example.com --set clusterName=prod-eu \
  --set existingSecret=observecraft-agent
```

The chart installs:

- a **DaemonSet** collecting container logs, kubelet pod and container metrics, and host metrics. It also exposes
  OTLP (4317/4318), dd-trace (8126) and DogStatsD (8125/udp) on each node's IP and through a node-local Service, and
  scrapes annotated Prometheus endpoints. Log read positions and the export queue are kept on the node, so restarts
  neither lose nor resend data.
- a **Deployment** for the cluster: cluster state metrics, the object snapshots behind the Kubernetes Explorer, and
  events.

Applications send to the collector on their node:

```yaml
env:
  - name: HOST_IP
    valueFrom: { fieldRef: { fieldPath: status.hostIP } }
  - name: OTEL_EXPORTER_OTLP_ENDPOINT   # OpenTelemetry SDKs
    value: http://$(HOST_IP):4318
  - name: DD_AGENT_HOST                 # Datadog tracers and DogStatsD
    value: $(HOST_IP)
```

Pod labels `tags.datadoghq.com/service|env|version` (or `OTEL_SERVICE_NAME` / `OTEL_RESOURCE_ATTRIBUTES`) set
service, env and version the same way Datadog's unified service tagging does.

**Hosts:** [`deploy/examples/otel-host/collector.yaml`](../deploy/examples/otel-host/collector.yaml) covers host
metrics, `/var/log/*.log`, OTLP, dd-trace and DogStatsD. Its header has the `docker run` line. Add receivers to it as
you need them, for example syslog:

```yaml
receivers:
  syslog:
    udp: { listen_address: 0.0.0.0:514 }
    protocol: rfc5424
```

## Fluent Bit

The `datadog` output (Datadog's logs API):

```ini
[OUTPUT]
    Name        datadog
    Match       *
    Host        intake.observecraft.example.com
    TLS         on
    compress    gzip
    apikey      ${OBSERVECRAFT_API_KEY}
    dd_service  storefront-api
    dd_source   nginx
    dd_tags     env:prod,team:web
```

## Fluentd

[fluent-plugin-datadog](https://github.com/DataDog/fluent-plugin-datadog):

```xml
<match **>
  @type datadog
  api_key "#{ENV['OBSERVECRAFT_API_KEY']}"
  host intake.observecraft.example.com
  port 443
  use_ssl true
  use_http true
  use_compression true
  dd_source app
</match>
```

## Vector

```yaml
sinks:
  observecraft_logs:
    type: datadog_logs
    inputs: [my_logs]
    endpoint: https://intake.observecraft.example.com
    default_api_key: "${OBSERVECRAFT_API_KEY}"
    compression: gzip
  observecraft_metrics:
    type: datadog_metrics
    inputs: [my_metrics]
    endpoint: https://intake.observecraft.example.com
    default_api_key: "${OBSERVECRAFT_API_KEY}"
```

Vector's distribution metrics go to `/api/beta/sketches` and are stored as exponential histograms.

## Logstash

```ruby
output {
  http {
    url => "https://intake.observecraft.example.com/api/v2/logs"
    http_method => "post"
    format => "json_batch"
    http_compression => true
    headers => { "Authorization" => "Bearer ${OBSERVECRAFT_API_KEY}" }
  }
}
```

## Filebeat

Filebeat can only send to Elasticsearch, Logstash, Kafka or Redis. Datadog doesn't accept Filebeat either; the
options are the same:

1. **Replace Filebeat** with the Datadog Agent's log collection, the OpenTelemetry Collector (`filelog` receiver),
   Fluent Bit or Vector. This is the long-term choice.
2. **Keep Filebeat** and put a small Vector in front of the intake. It receives the Beats (Lumberjack) protocol on
   5044, so Filebeat only changes its output:

   ```yaml
   # filebeat.yml
   output.logstash:
     hosts: ["vector-gateway:5044"]
   ```

   ```yaml
   # vector.yaml
   sources:
     beats:
       type: logstash
       address: 0.0.0.0:5044
   transforms:
     ecs:
       type: remap
       inputs: [beats]
       source: |
         # Filebeat's ECS objects → the reserved attributes ObserveCraft reads.
         if is_object(.host) { .host = .host.name }
         if is_object(.service) { .service = .service.name }
         .ddsource = .input.type
         if .ddsource == null { .ddsource = "filebeat" }
   sinks:
     observecraft:
       type: datadog_logs
       inputs: [ecs]
       endpoint: https://intake.observecraft.example.com
       default_api_key: "${OBSERVECRAFT_API_KEY}"
       compression: gzip
   ```

   The same works with an existing Logstash: `beats` input → `http` output, as above.

## Prometheus

- **Kubernetes:**
  - With the Datadog Agent, use its OpenMetrics autodiscovery (pod annotations `ad.datadoghq.com/…`).
  - With the `observecraft-agent` chart, annotate pods with `prometheus.io/scrape: "true"` (plus `prometheus.io/port`
    and `prometheus.io/path`). The collector on the pod's node scrapes them.
- **Elsewhere:** a Collector with the `prometheus` receiver and the `otlphttp/observecraft` exporter.

Prometheus `remote_write` isn't accepted, the same as Datadog.

## Limits and errors

- Each organization is rate limited (Helm: `intake.orgRequestsPerSecond`, `intake.orgMegabytesPerSecond`). Over the
  limit the intake answers `429` with `Retry-After` (gRPC: `UNAVAILABLE`); every sender above retries.
- A missing, invalid or revoked key gets `401` (Datadog's `/api/v1/validate`: `403`).
- Request bodies are limited to `intake.maxBodyMegabytes` (16 MB by default).
