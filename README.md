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
[Contributing](#contributing) · [License](#license)

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
- **Fast.** A Rust (axum) API over ClickHouse columnar storage. Search values are bound parameters, and live tail is
  pushed over NATS.
- **Enterprise-ready auth from day one.** Password + MFA, SAML 2.0, OpenID Connect, RBAC, application keys and an
  audit trail.
- **Multi-tenant.** Organizations keep their users, SSO, content and telemetry apart; one account can belong to
  several organizations and switch between them.
- **Self-hosted and MIT-licensed.** No per-host or per-GB pricing, and no data leaving your network.

## Features

| | Area | Highlights |
|---|---|---|
| 📜 | **Logs** | Query language with facets, histogram, live tail (SSE), anomaly insights, list/timeseries/top-list/table/tree-map/pie visualizations, trace ↔ log links |
| 🗂️ | **Log indexes** | Per-org indexes (Datadog model): filters route each log to the first matching index, each with its own retention, daily quota and sampled exclusion filters; Live Tail and archives still get every log |
| 🗄️ | **Log archives** | Hourly Parquet archives in your own S3-compatible bucket, Archive Search with DuckDB (no rehydration needed), rehydration back into the explorer |
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
| 🚀 | **Software Delivery** | CI pipeline explorer, flaky-test management, DORA metrics from OpenTelemetry CI/CD semantic conventions |
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

make up        # ClickHouse, Postgres, NATS, Vector, OTel Collector, RustFS (S3), synthetic logs
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

Logs written inside a trace (OTLP logs carry `trace_id`/`span_id`) link to it both ways: the span's **Logs** tab and the
log's **Trace** tab.

## Deploy on Kubernetes

The platform is one Helm chart, published with every release as an OCI artifact next to the image
(`ghcr.io/observecraft/observecraft`, amd64 and arm64). Pick a version from the
[releases](https://github.com/observecraft/observecraft/releases):

```bash
# Server (API + web app), intake, collector, Vector, and by default single-node ClickHouse, Postgres, NATS.
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
Keeper (Altinity operator), Postgres as a CloudNativePG cluster with failover, three NATS nodes, and autoscaled,
disruption-budgeted services. Or point the chart at your own databases (`clickhouse.mode=external`,
`postgres.mode=external`).

The chart generates and keeps its secrets (or uses `secrets.existingSecret`), runs everything as non-root with
read-only root filesystems, and restricts traffic between components with NetworkPolicies. Every server replica can
start at once: migrations run under a Postgres lock, and monitors and archiving run on one replica at a time.
ClickHouse gets a per-organization quota (`clickhouse.quota`). `helm test observecraft` checks the install.

Upgrade with `helm upgrade observecraft oci://ghcr.io/observecraft/charts/observecraft --version <new> --reuse-values`;
migrations run on startup. Read the release notes first: before 1.0, a minor version may change chart values.
Images and charts are signed and come with an SBOM and build provenance; [SECURITY.md](SECURITY.md#verifying-releases)
shows how to verify them. Every commit on `main` is also published as `:main` and `:sha-<commit>` images, for staging.

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
  IN -->|x-oc-org| COL[OTel Collector]
  IN -->|JSON logs| VEC[Vector aggregator]
  COL -->|logs| VEC
  COL --> CH[(ClickHouse<br/>logs · traces · metrics · CI<br/>per org)]
  VEC --> CH
  VEC --> NATS[[NATS<br/>live tail]]
  API[observecraft-server<br/>Rust · axum] --> CH
  API --> PG[(Postgres<br/>orgs · accounts · dashboards<br/>monitors · incidents)]
  API --> NATS
  API <-->|hourly Parquet| S3[(S3-compatible bucket<br/>log archives)]
  API -.->|DuckDB| S3
  API -->|webhook · Slack · email| OUT((Notifications))
  UI[Web app<br/>React · TypeScript] <-->|JSON · SSE| API
```

Every telemetry row carries its organization: the intake resolves the API key, the Collector stamps the resource with
`oc.org_id` (replacing anything a sender put there), and every query filters on it.

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
| `OC_QUERY_TIMEOUT_SECONDS` / `OC_QUERY_MAX_MEMORY_BYTES` | `60` / `4294967296` | Limits on each telemetry query; queries also send the org as ClickHouse `quota_key` for per-org quotas |
| `OC_POSTGRES_URL` | `postgres://observecraft:…@localhost:5432/observecraft` | Postgres connection string |
| `OC_NATS_URL` | `nats://localhost:4222` | NATS for live tail |
| `OC_PUBLIC_URL` | `http://localhost:5173` | Origin users open the app on (SSO redirects, login links, notification links) |
| `OC_SECRET_KEY` | — | **Required in production.** Encrypts MFA seeds, OIDC secrets, archive and channel credentials (`openssl rand -base64 48`) |
| `OC_INSTANCE_ADMIN_EMAIL` | — | Account that creates and manages organizations (becomes an admin of the install's org) |
| `OC_RETENTION_LOGS_DAYS` / `_TRACES_DAYS` | `15` / `15` | Defaults for a new org's first log index and its span retention (orgs change them in *Log Indexes* / *Data Retention*) |
| `OC_RETENTION_LOGS_MAX_DAYS` / `_TRACES_MAX_DAYS` | `365` / `90` | Most the instance admin can grant an org (*Instance Admin → Retention limits*) |
| `OC_RETENTION_METRICS_DAYS` / `_CI_DAYS` | `30` / `30` | Install-wide metric and CI retention |
| `OC_RAW_LOGS_HOURS` | `72` | Every log stays this long unindexed, as the source of archives |
| `OC_LOGPROC_ADDR` / `OC_INTERNAL_TOKEN` | `0.0.0.0:8090` / — | Internal log indexer Vector posts to (keep it off the public network; the token is optional) |
| `OC_SMTP_URL` / `OC_SMTP_FROM` | — | SMTP server for monitor emails (`smtps://user:pass@host:465`) |
| `OC_DATA_DIR` | `.oc-data` | Local cache for Archive Search and rehydration |
| `OC_COOKIE_SECURE` | `false` | Set `true` when served over HTTPS |
| `OC_SESSION_HOURS` | `12` | Idle session lifetime (sessions also end 7 days after sign-in) |
| `OC_WEB_DIST` | — | Serve the built web app from the API (production-style) |
| `OC_SEED_DEMO` | `true` | Fictional demo users, teams, monitors and incidents, a demo ingest key and archive. **Set `false` in production.** |

The intake (`oc-intake`) has its own settings: `OC_INTAKE_ADDR` (HTTP, `:4318`), `OC_INTAKE_GRPC_ADDR` (OTLP/gRPC,
`:4317`), `OC_INTAKE_COLLECTOR_URL`, `OC_INTAKE_LOGS_URL`, `OC_INTAKE_MAX_BODY_MB` (16), and per-organization limits
`OC_INTAKE_ORG_RPS` (500 requests/s) and `OC_INTAKE_ORG_MBPS` (50 MB/s).

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
| Monitors (metric, APM, log) with webhook / Slack / email notifications, downtimes, incidents | ✅ Live (Postgres) |
| Identity, SSO, audit trail | ✅ Live (Postgres) |
| CI visibility, flaky tests, DORA metrics | ✅ Live (ClickHouse) |
| Error tracking, integrations catalog | 🧪 UI with in-browser mocks |
| Log pipelines (parsing, remapping), logs → metrics, self-serve signup and per-org usage limits (SaaS mode) | 🗺️ Planned |
| Datadog Agent / Cluster Agent (Helm chart, Operator): metrics, distributions, logs, APM from dd-trace libraries, service checks, Kubernetes, host metadata, live processes and containers, events, continuous profiles, network connections (system-probe) | ✅ Live |
| PagerDuty / on-call | 🗺️ Planned |
| Live debugger, RUM | 🗺️ Planned |

## Contributing

Contributions are very welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, coding guidelines
and pull-request process. A quick check before you push:

```bash
make test       # Rust tests + web typecheck
make openapi    # if you changed an API endpoint
```

## License

ObserveCraft is released under the [MIT License](LICENSE).

<div align="center">
<sub>Built with Rust, React, ClickHouse and OpenTelemetry.</sub>
</div>
