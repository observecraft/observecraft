# ObserveCraft

[ObserveCraft](https://github.com/observecraft/observecraft) is an open-source, self-hosted observability platform:
logs, metrics, traces/APM, continuous profiling, dashboards, monitors, incidents, infrastructure, Kubernetes and network
monitoring, built on OpenTelemetry and ClickHouse. MIT-licensed.

Its intake speaks the **Datadog Agent and API formats** as well as **OTLP**, so you can keep the official Datadog Agent
(Helm chart or Operator), the dd-trace libraries and DogStatsD clients, or any OpenTelemetry SDK or Collector, and only
change the URL and API key.

This chart installs the whole platform:

| Component | What it does |
|---|---|
| `server` | API and web app; monitor evaluation, notifications, log archiving |
| `intake` | Public telemetry endpoint: OTLP (HTTP and gRPC), Datadog APIs; API key → organization, per-org rate limits |
| `collector` | OpenTelemetry Collector: batches per organization and writes traces, metrics and Kubernetes objects to ClickHouse |
| `vector` | Log normalization, then ClickHouse, the log indexer and live tail |
| ClickHouse | Telemetry storage: bundled single node, sharded and replicated through the Altinity operator, or your own |
| Postgres | Metadata (users, dashboards, monitors, …): bundled, CloudNativePG cluster, or your own |
| NATS | Live tail (best effort) |
| Valkey | Shared cache of the server replicas (required): bundled single instance without persistence, or your own |

## Install

```bash
helm install observecraft oci://ghcr.io/observecraft/charts/observecraft -n observecraft --create-namespace \
  --set publicUrl=https://observecraft.example.com \
  --set instanceAdminEmail=admin@example.com \
  --set ingress.enabled=true \
  --set ingress.app.host=observecraft.example.com \
  --set ingress.intake.host=intake.observecraft.example.com \
  --set ingress.certManagerClusterIssuer=letsencrypt
```

On first start the server prints a one-time link to set the instance admin's password:

```bash
kubectl -n observecraft logs deploy/observecraft-server | grep -i "set the admin password"
```

Then create an API key under *Organization Settings → API Keys* and point your senders at the intake host. `helm test
observecraft -n observecraft` checks the install.

Requirements: Kubernetes 1.26+, a default StorageClass (or `global.storageClass`), and an ingress controller for
external access.

## Send data

**Datadog Agent** (recommended for Kubernetes and hosts): install Datadog's own Helm chart (or Operator) with the intake as
its URL and an ObserveCraft API key as its key:

```bash
kubectl -n datadog create secret generic datadog-secret --from-literal api-key=ocik_…
helm repo add datadog https://helm.datadoghq.com
helm install datadog datadog/datadog -n datadog --create-namespace -f values.yaml --set datadog.clusterName=prod-eu
```

[`deploy/examples/datadog`](https://github.com/observecraft/observecraft/tree/main/deploy/examples/datadog) has the
`values.yaml` (every Agent product pointed at the intake, nothing sent to Datadog) and the same setup for the Datadog
Operator.

**OpenTelemetry**: any SDK or Collector, over OTLP/HTTP:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=https://intake.observecraft.example.com
OTEL_EXPORTER_OTLP_HEADERS="authorization=Bearer ocik_…"
```

For clusters without the Datadog Agent, the
[`observecraft-agent`](https://github.com/observecraft/observecraft/tree/main/deploy/helm/observecraft-agent) chart runs the OpenTelemetry Collector as a node and cluster agent. Every other sender (Fluent Bit, Vector, Logstash,
Prometheus, syslog, …) is in [docs/sending-data.md](https://github.com/observecraft/observecraft/blob/main/docs/sending-data.md).

## Production

The defaults run single-node ClickHouse, Postgres and Valkey inside the release: fine to try it and for small installs. For
production, use [`values-ha.yaml`](https://github.com/observecraft/observecraft/blob/main/deploy/helm/observecraft/values-ha.yaml)
or point the chart at databases you run:

```bash
# Replicated, sharded ClickHouse (Altinity operator) and a CloudNativePG cluster: install both operators first.
helm install observecraft oci://ghcr.io/observecraft/charts/observecraft -n observecraft --create-namespace \
  -f values-ha.yaml --set publicUrl=… --set instanceAdminEmail=…

# Your own databases (e.g. ClickHouse Cloud and Amazon RDS)
helm install observecraft oci://ghcr.io/observecraft/charts/observecraft -n observecraft --create-namespace \
  --set clickhouse.mode=external \
  --set clickhouse.external.httpUrl=https://clickhouse.example.internal:8443 \
  --set clickhouse.external.nativeAddress=clickhouse.example.internal:9440 \
  --set postgres.mode=external \
  --set cache.mode=external \
  --set secrets.existingSecret=observecraft-secrets   # keys: clickhouse-password, postgres-url, cache-url, …
```

### Sizing

Give every ClickHouse server (each replica of each shard) **at least 4 vCPU** and 16 GiB of memory: inserts, background
merges and dashboard queries share them, and with 2 vCPU merges and the metric rollup job compete with the UI as soon as
ingest grows. Development environments sized at 2 vCPU should move to 4. Add shards for more ingest: on the development
cluster (2 vCPU per server) one shard sustained about 200k points/s, and a second shard added about 1.4× that
(`IMPLEMENTATION_PLAN.md`, Stage 7); add replicas for more read capacity and availability. Set `clickhouse.bundled.resources` or `clickhouse.operator.resources`
(and `clickhouse.bundled.tuning.*` with the bundled server).

Security defaults: every pod runs as non-root with a read-only root filesystem and no capabilities; NetworkPolicies allow
only the traffic each component needs; secrets are generated once and kept across upgrades (or taken from
`secrets.existingSecret`). Server replicas start in any order: migrations run under a Postgres lock.

## Main settings

| Key | Default | Description |
|---|---|---|
| `publicUrl` | `""` | URL users open (SSO redirects, emailed links, cookies). Defaults to the ingress host. |
| `instanceAdminEmail` | `""` | Account that administers the install and creates organizations. |
| `defaultOrg` | `default` | Name of the first organization. |
| `image.tag` | chart `appVersion` | ObserveCraft image version. |
| `secrets.existingSecret` | `""` | Secret with `secret-key`, `internal-token`, `clickhouse-password`, `clickhouse-ingest-password`, `postgres-password`, `postgres-url`, `cache-password`, `cache-url`, `smtp-url` (missing keys are generated). |
| `retention.logsDays` / `tracesDays` / `metricsDays` / `ciDays` | `15` / `15` / `30` / `30` | Default retention. `logsMaxDays` / `tracesMaxDays` cap what the instance admin can grant an organization. |
| `retention.metrics5mDays` / `metrics1hDays` | `90` / `395` | How long metrics are kept as 5-minute and 1-hour rollups, which long ranges read. |
| `query.timeoutSeconds` / `maxMemoryBytes` / `maxRowsToRead` | `60` / `4 GiB` / `0` (none) | Limits on each ClickHouse query of an organization, so one can't starve the others. |
| `query.concurrencyPerOrg` / `queueTimeoutMs` | `16` / `10000` | Queries of one organization running at once per server replica; more wait up to the timeout, then get 429 (retry shortly). Background jobs (rollups, monitors) are exempt. |
| `smtp.url` / `smtp.from` | | Email for monitor notifications (put the URL in the secret key `smtp-url`). |
| `server.replicas`, `intake.replicas` | `2` | Plus `autoscaling.*`, `podDisruptionBudget.*`, `resources`, scheduling for each component. |
| `intake.orgRequestsPerSecond` / `orgMegabytesPerSecond` | `500` / `50` | Per-organization ingest limits (over them, senders get 429 and retry). |
| `clickhouse.mode` | `bundled` | `bundled`, `operator` (Altinity, sharded and replicated) or `external`. |
| `clickhouse.quota.*` | enabled | Per-organization ClickHouse query quota. |
| `clickhouse.bundled.coldStorage.*` | off | Moves metric samples and rollups older than `moveAfterDays` to an S3-compatible bucket (e.g. RustFS); they stay queryable. `endpoint` is the path-style bucket URL with a prefix, `existingSecret` holds `access-key-id` and `secret-access-key`. With `operator` or `external` ClickHouse, declare an `observecraft_tiered` storage policy (volumes `default` and `cold`) yourself and set `OC_METRICS_COLD_AFTER_DAYS` in `extraEnv`. |
| `clickhouse.bundled.tuning.*` | small server | Caches, concurrent queries, merge threads and async insert buffering for the bundled server. Raise them with `clickhouse.bundled.resources`. |
| `postgres.mode` | `bundled` | `bundled`, `operator` (CloudNativePG) or `external`. |
| `nats.enabled` | `true` | `false` to use `nats.external.url`. |
| `cache.mode` | `bundled` | `bundled` (one Valkey, `cache.bundled.resources`; maxmemory is `maxmemoryPercent` of the memory limit, least recently used keys go first) or `external`: any Valkey/Redis-protocol service (ElastiCache, Memorystore, …) at `cache.external.url` or the secret key `cache-url` (`redis://[user:password@]host:6379/0`, `rediss://` for TLS). |
| `cache.maxValueBytes` / `loadTimeoutSeconds` | `8 MiB` / `30` | Largest cached value (after compression); how long replicas wait for another replica's load of the same entry. |
| `ingress.enabled` | `false` | Hosts `ingress.app.host`, `ingress.intake.host`, optional `ingress.intakeGrpc` for OTLP/gRPC; TLS through cert-manager (`ingress.certManagerClusterIssuer`) or your own secrets. |
| `networkPolicy.enabled` | `true` | `networkPolicy.ingressFrom` limits who reaches the server and intake (your ingress controller). |
| `global.storageClass`, `global.imageRegistry`, `global.imagePullSecrets` | | Cluster-wide overrides. |

Every value is documented in [`values.yaml`](https://github.com/observecraft/observecraft/blob/main/deploy/helm/observecraft/values.yaml)
and validated by `values.schema.json`.

## Upgrade

```bash
helm upgrade observecraft oci://ghcr.io/observecraft/charts/observecraft --version <new version> -n observecraft --reuse-values
```

Database migrations run when the server starts. Before 1.0, a minor version may change chart values: read the
[changelog](https://github.com/observecraft/observecraft/blob/main/CHANGELOG.md) first.

## Verify the release

Charts and images are signed with Sigstore cosign from the release workflow; images also carry an SBOM and build
provenance:

```bash
cosign verify ghcr.io/observecraft/charts/observecraft:<version> \
  --certificate-identity-regexp '^https://github\.com/observecraft/observecraft/\.github/workflows/release\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## Uninstall

```bash
helm uninstall observecraft -n observecraft
kubectl -n observecraft delete pvc --all   # the volumes are kept by uninstall; this deletes all stored data
```
