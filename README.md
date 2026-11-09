<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="ObserveCraft" src="docs/assets/banner-light.svg" width="560">
</picture>

**Open-source, self-hosted observability platform: logs, metrics, traces, dashboards, monitors, incidents and CI
visibility in one place, built on OpenTelemetry and ClickHouse.**

[![CI](https://github.com/observecraft/observecraft/actions/workflows/ci.yml/badge.svg)](https://github.com/observecraft/observecraft/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/observecraft/observecraft?sort=semver)](https://github.com/observecraft/observecraft/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178c6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![React](https://img.shields.io/badge/React-18-61dafb?logo=react&logoColor=black)](https://react.dev)
[![ClickHouse](https://img.shields.io/badge/ClickHouse-storage-ffcc01?logo=clickhouse&logoColor=black)](https://clickhouse.com)
[![OpenTelemetry](https://img.shields.io/badge/OpenTelemetry-native-425cc7?logo=opentelemetry&logoColor=white)](https://opentelemetry.io)
<br>
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)
[![GitHub stars](https://img.shields.io/github/stars/observecraft/observecraft?style=flat&logo=github)](https://github.com/observecraft/observecraft/stargazers)
[![GitHub issues](https://img.shields.io/github/issues/observecraft/observecraft)](https://github.com/observecraft/observecraft/issues)
[![Last commit](https://img.shields.io/github/last-commit/observecraft/observecraft)](https://github.com/observecraft/observecraft/commits/main)

[Features](#features) · [Quickstart](#quickstart) · [Architecture](#architecture) · [Configuration](#configuration) ·
[Contributing](#contributing) · [Support](#support-observecraft) · [License](#license)

</div>

<br>

<p align="center">
  <img src="docs/assets/screenshots/trace-view.png" alt="Trace view with waterfall and span details" width="900">
</p>

## Why ObserveCraft?

- **One place for your telemetry.** Search logs, explore traces, chart metrics, alert on all of them and run incidents
  without stitching five tools together.
- **Open standards in, your storage underneath.** Send data with OpenTelemetry or Vector; it lands in ClickHouse
  tables you own and can query directly.
- **Fast and durable.** A Rust (axum) API over ClickHouse columnar storage. The intake writes what it accepts to
  Kafka before answering, so processing restarts and storage outages don't lose data, and live tail streams from it.
- **Enterprise-ready auth from day one.** Password + MFA, SAML 2.0, OpenID Connect, RBAC, application keys and an
  audit trail.
- **Multi-tenant.** Organizations keep their users, SSO, content and telemetry apart; one account can belong to
  several organizations and switch between them.
- **Self-hosted and MIT-licensed.** No per-host or per-GB pricing, and no data leaving your network.

## Features

| | Area | Highlights |
|---|---|---|
| 📜 | **Logs** | Query language with facets, histogram, live tail (SSE), anomaly insights, list/timeseries/top-list/table/tree-map/pie visualizations, trace ↔ log links |
| 🔀 | **Splunk migration** | Translate SPL searches step by step (filters, stats, timechart, top, macros) and import exported alerts and dashboards as log monitors and dashboards after a review |
| 🗂️ | **Log indexes** | Per-org indexes (Datadog model): filters route each log to the first matching index, each with its own retention, daily quota and sampled exclusion filters; Live Tail and archives still get every log |
| 🗄️ | **Log archives** | Hourly Parquet archives in your own S3-compatible bucket, Archive Search with DuckDB (no rehydration needed), rehydration back into the explorer with scan limits |
| 📤 | **Log forwarding** | Custom destinations: logs matching a query, after pipelines, forwarded to an HTTP endpoint, Splunk HEC or Elasticsearch with tag filtering; Flex Logs tier for longer index retention; all Logs Configuration resources managed with Datadog's Terraform provider (`datadog_logs_*`) |
| 🧭 | **APM** | Service catalog, endpoints, recommendations (N+1 queries, error hot spots), deployments, service pages with latency/error breakdowns |
| 🌐 | **Cloud Network Monitoring** | Traffic between clients and servers grouped by any tag (service, availability zone, namespace, pod, host, IP, port): volume, throughput, TCP retransmits, latency and jitter, connections; DNS view (requests, response time, timeouts, SERVFAIL, NXDOMAIN by domain); summary graphs, client/server facets, flow side panel, network map |
| 🔥 | **Continuous Profiler** | Datadog profiler uploads: CPU and memory per service over time, merged flame graphs with endpoint and package breakdowns, profile list, A/B comparison with differences coloured, CPU-burst insights with review status |
| 🔎 | **Trace explorer** | Span and trace search, facets, group-by analytics, flame graph, waterfall, span list, service map, span ↔ host metrics, logs and processes |
| 📈 | **Metrics** | Explorer and summary, query engine (`avg:system.cpu.user{env:prod} by {host}`), gauges, counters, histograms, exponential histograms and summaries (`http.server.request.duration.p95`), span-derived `trace.*` metrics |
| 🧩 | **Dashboards** | Drag-and-drop dashboards, lists, starring, built-in host and APM dashboards |
| 🖥️ | **Infrastructure** | Host list and map, host details with metrics, processes, pods, containers, spans and logs; Processes explorer; Event Management (All Events) |
| ☸️ | **Containers** | Kubernetes Overview and Explorer (pods, workloads, nodes, network, policies, storage, access control, CRDs, containers, container images) with facets, group-by, map and side panels (YAML, logs, APM, metrics, events, monitors); Resource Utilization; ECS Explorer (tasks, task definitions, services, clusters). Collector manifests in `deploy/examples/kubernetes`, ECS (FireLens + ADOT) in `deploy/examples/ecs` |
| 🚨 | **Monitors** | Metric, APM, log and change monitors; warn/alert thresholds with recovery; backtesting; downtimes; notifications to webhooks (HMAC-signed), Slack and email with retries; quality checks |
| 🧯 | **Incidents** | Declare, triage, timeline, impacts, follow-ups and analytics |
| 🚀 | **Software Delivery** | CI pipeline explorer, flaky-test management, DORA metrics from OpenTelemetry CI/CD semantic conventions; GitLab CI pipelines pulled through GitLab's API (no inbound access needed) |
| 🔐 | **Identity & tenancy** | Organizations with an instance admin console, one account across orgs, password + TOTP MFA, SAML 2.0, OIDC, roles, teams, SAML group mappings, application and ingest API keys, audit trail |

<details>
<summary><b>📸 More screenshots</b></summary>
<br>

**APM: services and recommendations**
<img src="docs/assets/screenshots/apm-services.png" alt="APM home" width="900">

**Trace explorer**
<img src="docs/assets/screenshots/trace-explorer.png" alt="Span explorer" width="900">

**Monitors**
<img src="docs/assets/screenshots/monitors.png" alt="Monitors list" width="900">

**Infrastructure**
<img src="docs/assets/screenshots/infrastructure.png" alt="Host list" width="900">

**CI/CD pipelines**
<img src="docs/assets/screenshots/ci-overview.png" alt="Pipelines overview" width="900">

</details>

## Quickstart

**Requirements:** Docker (Docker Desktop, OrbStack, Colima, …), Rust ≥ 1.89, Node.js ≥ 20, and `libxmlsec1` + `libxslt`
for SAML (`brew install libxmlsec1 libxslt` on macOS, `apt install libxmlsec1-dev libxml2-dev libxslt1-dev pkg-config`
on Debian/Ubuntu).

```bash
git clone https://github.com/observecraft/observecraft.git
cd observecraft
(cd web && npm install)

make up        # ClickHouse, Postgres, Kafka, Valkey, OTel Collector, RustFS (S3), synthetic logs
make server    # API on :8080 (applies migrations on startup)
make intake    # telemetry intake on :4318 (OTLP/HTTP + JSON logs, API-key authenticated)
make tracegen  # demo traces, logs, metrics and CI pipelines, sent through the intake
make web       # UI → http://localhost:5173
```

The demo generator also exports fictional Kubernetes telemetry for `shop-prod-eks` and
`shop-staging-eks`: nodes, pods, deployments, ReplicaSets, StatefulSets, DaemonSets, a completed
CronJob job and a healthy control plane. It sends compatibility metrics through the authenticated
OTLP intake every 10 seconds, so integration dashboards use the same storage/query path as real
telemetry. On startup it replays 30 minutes at one-minute resolution; set
`OC_K8S_BACKFILL_MINUTES=0` to disable replay (maximum: 60). Resources carry
`telemetry.synthetic=true`. The generator also sends kubelet service-check statuses every 10 seconds and workload
reconciliation events every minute. It does not produce Kubernetes object snapshots;
connect a real collector to populate the Kubernetes Explorer inventory.

Dashboard metric panels share a query plan. In live mode, a single authenticated
`POST /api/v1/query/dashboard/live` SSE connection returns the initial history and then
replaces the last two aligned buckets every 10 seconds. Compatible gauge/counter queries
share telemetry scans; distributions and span-derived metrics retain their own evaluators.
Identical subscriptions within an organization share the live plan and history. A paused view
uses `POST /api/v1/query/dashboard` once. Logs, events and distribution heatmaps use their
respective data-source endpoints. Shared metric frames include units from telemetry; display
conversions leave formula inputs unchanged. `exclude_null()` removes missing group tags while
retaining valid zero values. Event charts use the server's status histogram; integration tables
join formula columns by series group. Bundled notes contain ObserveCraft-authored guidance and
illustrations rather than upstream marketing copy or artwork.

The dashboard `weighted()` modifier accepts gauge sums and weights transient series using their
observed regular submission cadence, including incomplete live buckets. Without enough samples
to infer a cadence it keeps the unweighted sum. This is inferred from telemetry, not configured
submission-interval metadata. `top()` supports bounded ranking by a requested aggregation.
Forecasts, historical-shift formulas, and unsupported panel types remain explicit errors.

On first start the server logs a one-time link to set the admin password:

```
No one can sign in yet. Set the admin password for … : http://localhost:5173/login/link/…
```

That account (`OC_INSTANCE_ADMIN_EMAIL`) is also the **instance admin**: *account menu → Instance Admin* creates
organizations and invites their first admins. The demo org archives its logs to the local RustFS bucket
(console on `http://localhost:9101`).

**Other ways to run:**

- **Kubernetes (Helm):** see [Deploy on Kubernetes](#deploy-on-kubernetes).
- **Everything in containers:** `docker compose -f deploy/docker-compose.yml --profile app up -d` runs the published
  image (`ghcr.io/observecraft/observecraft`, set `OC_VERSION` to pin a release; built from source when it can't be
  pulled) with the API, the web app (served on `:8080`), the intake (`:4318`) and the demo generator. Add `--profile tls`
  (with `OC_DOMAIN` and `OC_INTAKE_DOMAIN`) for automatic HTTPS through Caddy.
- **UI only, no backend:** `make web-mock` runs the UI against in-browser mocks.
- **Try SSO locally:** `docker compose -f deploy/docker-compose.yml --profile sso up -d` starts a SAML IdP
  (SimpleSAMLphp on `:8081`, user `user1` / `user1pass`) and an OIDC provider (issuer `http://localhost:8082/default`).
  Configure them under *Organization Settings → Login Methods*.

### Send your own data

Create an API key under *Organization Settings → API Keys*. Everything sent with it belongs to that organization.
The intake accepts what Datadog accepts, so you can keep Datadog's open-source agents and libraries:

- **OTLP**, over HTTP (`:4318`) and gRPC (`:4317`), for traces, metrics and logs;
- **Datadog's API and Agent formats**: metrics, distributions, logs, APM traces, service checks, and the Cluster
  Agent's Kubernetes manifests.
- **OpenFeature flag evaluation** (OFREP, `/ofrep/v1/evaluate/flags`): any OpenFeature SDK with the OFREP provider
  evaluates the flags managed under *Software Delivery → Feature Flags*.

The **Datadog Agent** (official Helm chart or Operator) and Datadog's tracing libraries (dd-trace-*) and DogStatsD
clients need only a new URL and key. Fluent Bit, Fluentd, Vector and Logstash work through their Datadog outputs.

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_HEADERS="authorization=Bearer ocik_…"
export OTEL_SERVICE_NAME=my-service
export OTEL_RESOURCE_ATTRIBUTES=deployment.environment=dev,service.version=1.0.0
```

```bash
curl -X POST http://localhost:4318/api/v2/logs -H "DD-API-KEY: ocik_…" -H "content-type: application/json" \
  -d '[{"message":"payment declined","service":"checkout","status":"error","ddtags":"env:dev"}]'
```

For Kubernetes clusters and hosts, run the **Datadog Agent** (its Helm chart or Operator) pointed at the intake, or, for
an OpenTelemetry-only setup, the `observecraft-agent` Helm chart (the official OpenTelemetry Collector, configured).
**[docs/sending-data.md](docs/sending-data.md)** has the setup for every sender: the Datadog Agent and tracers, DogStatsD,
OpenTelemetry, Fluent Bit, Fluentd, Vector,
Logstash, Filebeat, Prometheus and syslog.

Configuration can be managed as code with **Datadog's Terraform provider** and API clients pointed at the server
(`api_url`, an application key `ocak_…`, optionally an API key `ocik_…`); support grows resource by resource, see
**[docs/datadog-api.md](docs/datadog-api.md)**.

Logs written inside a trace (OTLP logs carry `trace_id`/`span_id`) link to it both ways: the span's **Logs** tab and the
log's **Trace** tab.

## Deploy on Kubernetes

The platform is one Helm chart, published with every release as an OCI artifact next to the image
(`ghcr.io/observecraft/observecraft`, amd64 and arm64). Pick a version from the
[releases](https://github.com/observecraft/observecraft/releases):

```bash
# Server (API + web app, log ingester), intake, collector, and by default single-node ClickHouse, Postgres, Kafka, Valkey.
helm install observecraft oci://ghcr.io/observecraft/charts/observecraft --version 0.1.0 -n observecraft --create-namespace \
  --set publicUrl=https://observecraft.example.com --set instanceAdminEmail=admin@example.com \
  --set ingress.enabled=true --set ingress.app.host=observecraft.example.com \
  --set ingress.intake.host=intake.observecraft.example.com --set ingress.certManagerClusterIssuer=letsencrypt
```

In every cluster you monitor, install the Datadog Agent with Datadog's own Helm chart (or Operator), pointed at the
intake:

```bash
kubectl -n datadog create secret generic datadog-secret --from-literal api-key=ocik_…
helm install datadog datadog/datadog -n datadog -f deploy/examples/datadog/values.yaml --set datadog.clusterName=prod-eu
```

For an OpenTelemetry-only setup, use the `observecraft-agent` chart instead. It runs the official OpenTelemetry
Collector as a node agent and a cluster agent.

For production, [`values-ha.yaml`](deploy/helm/observecraft/values-ha.yaml) runs ClickHouse sharded and replicated with
Keeper (Altinity operator), Postgres as a CloudNativePG cluster with failover, three Kafka brokers (replication 3), and autoscaled,
disruption-budgeted services. Or point the chart at your own databases (`clickhouse.mode=external`,
`postgres.mode=external`) and cache (`cache.mode=external`: ElastiCache, Memorystore or any Valkey/Redis-protocol
server).

The chart generates and keeps its secrets (or uses `secrets.existingSecret`), runs everything as non-root with
read-only root filesystems, and restricts traffic between components with NetworkPolicies. Every server replica can
start at once: migrations run under a Postgres lock, and monitors and archiving run on one replica at a time.
ClickHouse gets a per-organization quota (`clickhouse.quota`). `helm test observecraft` checks the install.

Upgrade with `helm upgrade observecraft oci://ghcr.io/observecraft/charts/observecraft --version <new> --reuse-values`;
migrations run on startup. Read the release notes first: before 1.0, a minor version may change chart values.
Images and charts are signed and come with an SBOM and build provenance; [SECURITY.md](SECURITY.md#verifying-releases)
shows how to verify them. Every commit on `main` is also published as `:main` and `:sha-<commit>` images, for staging.


### Monitoring ObserveCraft itself

The instance admin's organization (or `selfMonitoring.org`) gets an **ObserveCraft Overview** dashboard and
recommended monitors tagged `observecraft:self-monitoring`, over metrics the server measures every minute:

- **ClickHouse**: disk use and growth, days until full, each table's size, parts per partition (inserts slow down at
  1,000 and fail at 3,000), seconds since each signal was last written, failing mutations, replication delay;
- **Kafka**: the log ingester's lag; **Postgres**: connections against `max_connections`, database size; overdue
  cloud crawl jobs;
- **cost**: the install's containers at the organization's Kubernetes prices (Kubernetes → Autoscaling → Settings;
  needs the Datadog Agent on ObserveCraft's cluster, reporting to the operator's organization) plus disk, and the next
  30 days at the current growth; **usage**: each organization's stored events per signal and their estimated bytes.

Monitors alert before things break (disk at 75 %/85 %, full within 14/7 days, no data written for 5/15 minutes, log
lag, Postgres connections, too many parts); set `selfMonitoring.notify` to the handles they notify. They can't
report that ObserveCraft itself is down: set `selfMonitoring.heartbeatUrl` to a dead man's switch outside the
install (a healthchecks.io check, Better Stack, Cronitor), which alerts when the calls stop.

## Architecture

```mermaid
flowchart LR
  subgraph Sources
    A[Apps & services<br/>OpenTelemetry SDKs]
    H[Clusters & hosts<br/>ObserveCraft Agent]
    C[CI pipelines<br/>OTel CI/CD spans]
  end
  D[Datadog Agent · Fluent Bit<br/>Vector · Logstash]
  A & H & C -->|OTLP HTTP/gRPC + API key| IN[oc-intake<br/>auth · rate limits]
  D -->|Datadog logs / series API| IN
  IN -->|acks=all · keyed by org| K[[Kafka<br/>durable buffer · 72 h]]
  K -->|traces · metrics · signals| COL[OTel Collector]
  K -->|logs| API[observecraft-server<br/>Rust · axum<br/>log ingester]
  COL --> CH[(ClickHouse<br/>logs · traces · metrics · CI<br/>per org)]
  API --> CH
  API --> PG[(Postgres<br/>orgs · accounts · dashboards<br/>monitors · incidents)]
  API -->|live tail| K
  API <-->|shared results| VK[(Valkey<br/>cache)]
  API <-->|hourly Parquet| S3[(S3-compatible bucket<br/>log archives)]
  API -.->|DuckDB| S3
  API -->|webhook · Slack · email| OUT((Notifications))
  UI[Web app<br/>React · TypeScript] <-->|JSON · SSE| API
```

Every telemetry row carries its organization: the intake resolves the API key and writes the org as a Kafka record
header, the Collector stamps the resource with `oc.org_id` (replacing anything a sender put there), and every query
filters on it. The intake answers a sender once Kafka has its data on all in-sync replicas; the Collector and the
server's log ingester move their Kafka offsets only after storing (see [ADR-0004](docs/adr/0004-kafka-ingestion-buffer.md)
and [ADR-0005](docs/adr/0005-kafka-log-ingester.md)).

| Path | What |
|---|---|
| `web/` | React + Vite + TypeScript UI. `src/api/*/client.ts` is the only data-access layer. |
| `server/` | Rust workspace: `oc-query` (search syntax → ClickHouse/DuckDB SQL, matcher), `oc-store` (ClickHouse + Postgres, migrations, retention), `oc-api` (`observecraft-server`), `oc-intake` (telemetry gateway: OTLP HTTP/gRPC, Datadog APIs), `oc-tracegen` (synthetic data). |
| `server/migrations/` | ClickHouse (telemetry) and Postgres (metadata) schemas. |
| `deploy/` | Docker Compose stack, Helm charts (`helm/observecraft`, `helm/observecraft-agent`), Vector aggregator (`vector/normalize.vrl`, with unit tests) + load generator, OTel Collector config (shared by Compose and Helm), RustFS, Caddy (TLS), Kubernetes and ECS examples (`examples/`). |

### API contract

The OpenAPI document is generated from the Rust handlers (utoipa), and the web's TypeScript types are generated from
it: run **`make openapi`** after changing an endpoint. `web/src/api/http.ts` checks paths, bodies and responses against
`web/src/api/generated/schema.d.ts` at compile time. The live spec is served at `GET /api/openapi.json`.

## Configuration

The server reads its settings from environment variables; see [`.env.example`](.env.example) for the full list. The
most important ones:

| Variable | Default | Description |
|---|---|---|
| `OC_HTTP_ADDR` | `0.0.0.0:8080` | API listen address |
| `OC_CLICKHOUSE_URL` | `http://localhost:8123` | ClickHouse HTTP endpoint (`OC_CLICKHOUSE_USER` / `_PASSWORD`) |
| `OC_CLICKHOUSE_CLUSTER` | — | ClickHouse cluster name (from `remote_servers`, with Keeper): tables are created replicated and sharded on it. Unset for a single server |
| `OC_RUN_MIGRATIONS` | `true` | Apply migrations at startup (under a Postgres lock, so replicas can start together). `false` when a deploy step runs `observecraft-server migrate` |
| `OC_CLICKHOUSE_DROP_LEGACY_SCHEMA` | `false` | Allow replacing a ClickHouse schema from before the org-sorted tables (deletes its telemetry) |
| `OC_ORG_QUERY_TIMEOUT_SECONDS` / `OC_ORG_QUERY_MAX_MEMORY_BYTES` / `OC_ORG_QUERY_MAX_ROWS_TO_READ` | `60` / `4294967296` / `0` (none) | Limits on each telemetry query of an organization (`OC_QUERY_TIMEOUT_SECONDS` / `OC_QUERY_MAX_MEMORY_BYTES` still work); queries also send the org as ClickHouse `quota_key` for per-org quotas. A query stopped at a limit gets 503 with what to change |
| `OC_ORG_QUERY_CONCURRENCY` / `OC_ORG_QUERY_QUEUE_TIMEOUT_MS` | `16` / `10000` | ClickHouse queries of one organization running at once on each server replica (0 = no limit); more wait up to the timeout, then the API answers 429 ("retry shortly"). Background jobs (metric rollups, monitors, schedulers) are exempt |
| `OC_POSTGRES_URL` | `postgres://observecraft:…@localhost:5432/observecraft` | Postgres connection string |
| `OC_KAFKA_BROKERS` | `localhost:19092` | Kafka bootstrap brokers (`host:port,…`). The server ingests logs and reads live tail from it; the intake (`oc-intake`) writes telemetry to it, and forwards over HTTP to the Collector and Vector when it is unset |
| `OC_LOG_INGEST` | `kafka` | `kafka`: the server reads the log topics itself ([ADR-0005](docs/adr/0005-kafka-log-ingester.md); `OC_LOG_INGEST_CONCURRENCY` batches at once, by default the CPUs it may use, 4–16; switching from `vector`, it
starts `OC_LOG_INGEST_HANDOVER_MINUTES`, 30, before the newest log Vector stored). `vector`: the earlier path through the Vector aggregator (Compose `--profile vector`), needed by an intake without Kafka |
| `OC_CACHE_URL` | — | **Required.** Shared cache of all server replicas: Valkey (or another Redis-protocol server), `redis://[user:password@]host:6379/0` or `rediss://…` for TLS. The server exits at startup when it is unset or unreachable for 30 s; later outages only cost cache hits |
| `OC_DATADOG_API_ALLOW_APP_KEY_CREATION` | `false` | Lets the Datadog-compatible API create application keys when the caller is itself an application key (Terraform's `datadog_application_key`). Off by default so a leaked key can't create keys that outlive its revocation; see [docs/datadog-api.md](docs/datadog-api.md) |
| `OC_CACHE_MAX_VALUE_BYTES` / `OC_CACHE_LOAD_TIMEOUT_SECONDS` | `8388608` / `30` | Largest value stored (after compression); how long replicas wait for another replica's load of the same entry |
| `OC_INVENTORY_CACHE_MAX_OBJECTS` | `500000` | Kubernetes objects the in-process inventory cache keeps over all orgs |
| `OC_PUBLIC_URL` | `http://localhost:5173` | Origin users open the app on (SSO redirects, login links, notification links) |
| `OC_AWS_PRINCIPAL_ARN` | unset | The AWS IAM role the server assumes customers' AWS integration roles with; their roles trust it (Integrations → Amazon Web Services) |
| `OC_GCP_PRINCIPAL_EMAIL` | unset | The GCP service account the server impersonates customers' service accounts with (the STS delegate); they grant it Service Account Token Creator |
| `OC_SLACK_CLIENT_ID`, `OC_SLACK_CLIENT_SECRET` | unset | This install's Slack app, for "Add to Slack" in Integrations → Slack (without them, workspaces connect with their own app's bot token) |
| `OC_SLACK_SIGNING_SECRET` | unset | That app's signing secret: verifies the mute buttons of Slack alerts |
| `OC_CLOUD_CRAWLERS` | `aws,azure,gcp` | The cloud integrations whose metrics the server crawls (CloudWatch, Azure Monitor, Cloud Monitoring); needs `OC_KAFKA_BROKERS` |
| `OC_CLOUD_CRAWL_INTERVAL_SECS` | `600` | How often each account's region, subscription or project is read |
| `OC_CLOUD_CRAWL_CONCURRENCY` | `8` | Crawl jobs one server runs at a time per cloud (add servers to crawl more) |
| `OC_SECRET_KEY` | — | **Required in production.** Encrypts MFA seeds, OIDC secrets, archive and channel credentials (`openssl rand -base64 48`) |
| `OC_INSTANCE_ADMIN_EMAIL` | — | Account that creates and manages organizations (becomes an admin of the install's org) |
| `OC_RETENTION_LOGS_DAYS` / `_TRACES_DAYS` | `15` / `15` | Defaults for a new org's first log index and its span retention (orgs change them in *Log Indexes* / *Data Retention*) |
| `OC_RETENTION_LOGS_MAX_DAYS` / `_TRACES_MAX_DAYS` | `365` / `90` | Most the instance admin can grant an org (*Instance Admin → Retention limits*) |
| `OC_RETENTION_METRICS_DAYS` / `_CI_DAYS` | `30` / `30` | Install-wide metric and CI retention |
| `OC_RAW_LOGS_HOURS` | `72` | Every log stays this long unindexed, as the source of archives |
| `OC_LOGPROC_ADDR` / `OC_INTERNAL_TOKEN` | `0.0.0.0:8090` / — | Internal listener: the log indexer Vector posts to (`OC_LOG_INGEST=vector`), and the server's own metrics (`GET /metrics`, Prometheus text: `oc_cache_*`). Keep it off the public network; the token is optional |
| `OC_SMTP_URL` / `OC_SMTP_FROM` | — | SMTP server for monitor emails (`smtps://user:pass@host:465`) |
| `OC_ASSIST_PROVIDER` | — | The install's default model for Assist: `anthropic` (the Messages API) or `openai` (any OpenAI-compatible chat completions endpoint: OpenAI, Azure OpenAI, vLLM, LiteLLM, Ollama). Each organization can set its own in Organization Settings → Assist AI (key sealed), which replaces this one for it. Unset and no organization provider: Assist says it isn't set up |
| `OC_ASSIST_API_KEY` / `OC_ASSIST_MODEL` / `OC_ASSIST_BASE_URL` | — / `claude-opus-5` (anthropic) / `https://api.anthropic.com` | The provider's key (required for anthropic), model (required for openai) and URL (required for openai, e.g. `https://api.openai.com/v1`) |
| `OC_ASSIST_ALLOW_PRIVATE` | `false` | Lets organizations point Assist at private addresses (a model on the install's network); leave off on multi-tenant installs |
| `OC_ASSIST_FALLBACKS` / `OC_ASSIST_MAX_TOKENS` | `true` / 64000 (anthropic), 4096 (openai) | Anthropic's server-side fallback when the model declines a request; the most a model turn writes |
| `OC_SYNTHETICS_ALLOW_PRIVATE` | `false` | Let synthetic tests reach loopback, private, link-local (cloud metadata) and other internal addresses |
| `OC_DATA_DIR` | `.oc-data` | Local cache for Archive Search and rehydration |
| `OC_COOKIE_SECURE` | `false` | Set `true` when served over HTTPS |
| `OC_SESSION_HOURS` | `12` | Idle session lifetime (sessions also end 7 days after sign-in) |
| `OC_WEB_DIST` | — | Serve the built web app from the API (production-style) |
| `OC_LOG_FORWARD_ALLOW_PRIVATE` | `false` | Custom destinations may forward logs to private and loopback addresses |
| `OC_SCANNER_TOKEN` | — | Bearer token of Cloud Security's account scanners (16+ characters; the `scanner` service / `scanner.enabled`). Without it no account is scanned |
| `OC_POSTURE_SCAN_INTERVAL_HOURS` | `24` | Hours between two Prowler scans of a cloud account |
| `OC_KEV_URL` | CISA's catalog | Known Exploited Vulnerabilities JSON the Security research feed reads every six hours (`off` without internet access) |
| `OC_SELF_MONITORING` | `true` | ObserveCraft monitoring itself: every minute one server writes `observecraft.*` metrics (ClickHouse disks, parts and freshness, the log ingester's Kafka lag, Postgres, jobs, cost, each organization's usage) into the operator's organization, with the *ObserveCraft Overview* dashboard and recommended monitors there; see [Monitoring ObserveCraft itself](#monitoring-observecraft-itself) |
| `OC_SELF_MONITORING_ORG` / `OC_SELF_MONITORING_NOTIFY` | first instance admin's org / — | The operator's organization; the handles its recommended monitors notify (`@slack-ops @ops@example.com`) |
| `OC_HEARTBEAT_URL` | — | Called every minute while Postgres and ClickHouse answer (e.g. a healthchecks.io check), so a service outside alerts when ObserveCraft is down |
| `OC_SELF_MONITORING_NAMESPACE` / `OC_SELF_MONITORING_STORAGE_GB_MONTH` | `observecraft` / `0.08` | Kubernetes namespace of the install (its containers' cost, from the Datadog Agent's `container.*`); USD per GB-month of disk |
| `OC_SEED_DEMO` | `true` | Fictional demo users, teams, monitors and incidents, a demo ingest key and archive. **Set `false` in production.** |

The intake (`oc-intake`) has its own settings: `OC_INTAKE_ADDR` (HTTP, `:4318`), `OC_INTAKE_GRPC_ADDR` (OTLP/gRPC,
`:4317`), `OC_INTAKE_COLLECTOR_URL`, `OC_INTAKE_LOGS_URL`, `OC_INTAKE_AUTOSCALING_URL` (the server's internal listener for the
Datadog Cluster Agent's external recommender calls), `OC_INTAKE_MAX_BODY_MB` (16), `OC_INTAKE_SERIES_CACHE` (6,000,000: series a replica remembers as forwarded in the
hour, ~20 bytes each; keep it above the active series it sees), and per-organization limits
`OC_INTAKE_ORG_RPS` (requests/s) and `OC_INTAKE_ORG_MBPS` (MiB/s), unlimited (`0`) by default; instance admins can
also limit one organization (Instance → Organizations → Limits).

## Security

- **Tenancy:** every Postgres row and every ClickHouse telemetry row carries its organization; a request's org comes
  only from its session or application key, a sender's org only from its ingest API key — never from client input.
  An integration test (`server/crates/oc-api/tests/tenancy.rs`) checks isolation end to end.
- **Accounts across orgs:** links issued by one org's admins can never set the password of an account that also
  belongs to another org; sessions started with SSO stay in that org.
- **Queries:** all search values *and* tag keys are bound parameters (`oc-query::build_where`), never interpolated into
  SQL.
- **Authentication:** Argon2id passwords, TOTP MFA, account lockout, signed SAML responses (SHA-256+, replay
  protection), OIDC with PKCE and nonce, HttpOnly SameSite cookies with origin checks on writes.
- **Roles:** Admin, Standard and Read Only, enforced by the API.

Found a vulnerability? Please report it privately; see [CONTRIBUTING.md](CONTRIBUTING.md#reporting-security-issues).

## Project status

ObserveCraft is under active development and **not yet production-ready**: APIs and schemas may still change between
releases.

| Area | Status |
|---|---|
| Ingest (OTLP HTTP/gRPC, Datadog logs and metrics APIs), API keys, organizations | ✅ Live |
| Helm charts (platform; OpenTelemetry Collector agent), Datadog Agent / Operator examples | ✅ Live |
| Logs, live tail, log indexes (retention, quotas, exclusions), archives, Archive Search, rehydration, trace ↔ log links | ✅ Live (ClickHouse, S3 + DuckDB) |
| APM, trace explorer, metrics (all OTLP types), dashboards, infrastructure | ✅ Live (ClickHouse) |
| APM settings: span flow overview, default environment and primary tags, ingestion per service, span-based metrics and retention filters (Terraform `datadog_spans_metric`, `datadog_apm_retention_filter` / `_order`), span pipelines | ✅ Live (ClickHouse, Postgres); retention filters and pipelines not applied to incoming spans yet |
| Monitors (metric, APM, log) with webhook / Slack / email notifications, downtimes, incidents | ✅ Live (Postgres) |
| Identity, SSO, audit trail | ✅ Live (Postgres) |
| CI visibility, flaky tests, DORA metrics | ✅ Live (ClickHouse) |
| Error tracking, integrations catalog | 🧪 UI with in-browser mocks |
| Log pipelines (parsing, remapping), logs → metrics, self-serve signup and per-org usage limits (SaaS mode) | 🗺️ Planned |
| Datadog Agent / Cluster Agent (Helm chart, Operator): metrics, distributions, logs, APM from dd-trace libraries, service checks, Kubernetes, host metadata, live processes and containers, events, continuous profiles, network connections (system-probe) | ✅ Live |
| PagerDuty / on-call | 🗺️ Planned |
| RUM: applications (Terraform `datadog_rum_application`), browser and mobile SDK events, Summary, Explorer | ✅ Live (ClickHouse, Postgres) |
| RUM Session Replay: browser recordings, player, playlists, watch history, heatmaps; Product Analytics | ✅ Live (ClickHouse, Postgres) |
| Cloud SIEM: content packs (CloudTrail, Okta, GitHub, Kubernetes audit, web servers, …), log detection rules and suppressions (Terraform `datadog_security_monitoring_rule` / `_suppression`), signals and triage; Security overview, inbox and research feed (CISA KEV) | ✅ Live (ClickHouse, Postgres) |
| Cloud Security: host and container image vulnerabilities from the Agent's SBOMs, Kubernetes and host misconfigurations, inventory, infrastructure packages, CVE explorer | ✅ Live (Postgres, ClickHouse) |
| App & API Protection: security traces from the tracers' AppSec spans, attackers and users, API endpoints and findings (OWASP API Top 10), In-App WAF custom rules, exclusion filters and policies (Terraform `datadog_appsec_waf_custom_rule` / `_exclusion_filter`), denylist and detection rules | ✅ Live (ClickHouse, Postgres); not yet pushed to tracers |
| Workload Protection: the Agent's runtime security events, agent policies and rules (Terraform `datadog_csm_threats_policy` / `_agent_rule`), Agents and coverage, `workload_security` detection rules and signals, `workload_activity` findings, MITRE ATT&CK coverage | ✅ Live (ClickHouse, Postgres); policies reach Agents as downloaded files |
| AI Guard: the tracers' AI Guard SDK evaluate endpoint with our own evaluator (prompt injection, jailbreak, prompt extraction, exfiltration, destructive tool calls, sensitive data), Investigate with the conversation timeline, Signals, Discover (AI services and model providers from LLM spans), Playground, per-service blocking | ✅ Live (ClickHouse, Postgres) |
| Live debugger | 🗺️ Planned |

## Contributing

Contributions are very welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, coding guidelines
and pull-request process. A quick check before you push:

```bash
make test       # Rust tests + web typecheck
make openapi    # if you changed an API endpoint
```

## Support ObserveCraft

[Sponsor @andreimerfu](https://github.com/sponsors/andreimerfu), ObserveCraft's maintainer, to help fund
open-source development, CI and releases, and documentation.

You can also support the project by reporting issues, improving documentation and contributing fixes.

## License

ObserveCraft is released under the [MIT License](LICENSE).

<div align="center">
<sub>Built with Rust, React, ClickHouse and OpenTelemetry.</sub>
</div>
