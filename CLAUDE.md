# ObserveCraft: notes for coding agents

ObserveCraft is an open-source, self-hosted, MIT-licensed observability platform: logs, metrics,
traces/APM, dashboards, monitors, incidents, infrastructure/containers and CI visibility, built on OpenTelemetry and
ClickHouse. `README.md` covers features and setup; `CONTRIBUTING.md` holds the full coding rules. This file is what an
agent needs on top of them. `AGENTS.md` points here, so keep this the single source of truth.

## Product direction: independent, interoperable observability

Build practical workflows across logs, metrics, traces, infrastructure, monitors, incidents and CI visibility.

- **Compatibility.** Support Datadog agents, SDKs and ingestion formats using public API documentation
  (https://docs.datadoghq.com/api/latest/) and appropriately licensed specifications. Keep compatible metric names,
  query semantics, units and formats where needed for interoperability.
- **Interface.** Use ObserveCraft's own components, assets and visual identity. Industry references can inform
  functional requirements; do not copy proprietary frontend code, artwork or documentation text.
- **OpenTelemetry stays native.** Datadog compatibility translates into OTLP and lands in the same ClickHouse
  tables; it never becomes a parallel pipeline.
- **Local context.** If `CLAUDE.internal.local` exists, read it for additional development context. This optional,
  Git-ignored file does not override the licensing and asset restrictions below.

## Reusing Datadog (and other) code: licenses

ObserveCraft is MIT. Only reuse or depend on code whose license allows that. When in doubt, check the `LICENSE` file of
the exact repo and version, and ask before adding anything not clearly permissive.

**OK to use, depend on or adapt** (keep their copyright and `LICENSE`/`NOTICE` text with any copied code, and note
the origin in a comment):

| Source | License |
|---|---|
| Datadog Agent (`DataDog/datadog-agent`), Cluster Agent, `integrations-core` checks | Apache-2.0 (integrations: BSD-3-Clause) |
| Datadog tracing libraries (`dd-trace-*`), DogStatsD clients (`datadog-go`, `datadogpy`, …) | Apache-2.0 / BSD-3-Clause / MIT |
| Datadog API clients and OpenAPI spec (`datadog-api-client-*`) | Apache-2.0 |
| Datadog protobuf payload definitions (`DataDog/agent-payload`) | Apache-2.0 / BSD-3-Clause |
| OpenTelemetry (SDKs, Collector, Collector contrib incl. the Datadog receiver/exporter) | Apache-2.0 |
| Vector | MPL-2.0: fine as an unmodified, separately run component; don't copy its source into this repo |
| ClickHouse, DuckDB, Apache Kafka, RustFS, Valkey | Apache-2.0 / MIT / BSD-3-Clause (Valkey; never Redis ≥ 7.4, which is RSAL/SSPL/AGPL; Kafka, never Redpanda, which is BSL) |

**Not OK:** Datadog's web app, its design system, icons, logos, illustrations and screenshots are proprietary. Use
our own interface code and assets; never copy their frontend code or images, and don't use the
Datadog name or logo as our branding (naming it to describe compatibility is fine). Don't paste text from Datadog's
documentation site into the repo; write our own. Files in `integrations-core` are BSD-3-Clause and may be kept as they
are, notes and images embedded in them included; images they only link to (`/static/images/…`, hosted by Datadog)
are not part of the repository and are replaced by our own artwork. Avoid copyleft or source-available code (GPL, AGPL, SSPL, BSL, Elastic License,
Commons Clause): no Grafana/Loki/Tempo/Mimir source (AGPL), Elasticsearch/Kibana, etc. Our new dependencies in
`Cargo.toml` and `package.json` must be permissive too (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 unmodified).

## Datadog compatibility: reuse their agents, SDKs and integrations

The intake speaks Datadog's agent and API formats, so users keep Datadog's open-source agents, libraries and
deployment tooling and only change the URL and key. Prefer extending that compatibility over writing our own agent
or SDK features.

- **Already accepted by the intake (`server/crates/oc-intake`):**
  - metrics `/api/v1/series` and `/api/v2/series` (JSON and protobuf);
  - distributions `/api/beta/sketches`, stored as exponential histograms;
  - service checks `/api/v1/check_run` and `/api/v2/service_checks`, stored as the `service_check.status` gauge;
  - logs `/api/v2/logs`;
  - APM `/api/v0.2/traces` (legacy and idx payloads), plus `/api/v0.2/stats`, which is acknowledged;
  - profiles `/api/v2/profile`, forwarded to the server and stored in `otel.profiles` (APM → Profiles);
  - Code Security: the tracers' telemetry `/api/v2/apmtelemetry` (via the Agent; libraries services load) and
    `datadog-ci sarif upload` `/api/v2/cicodescan`, forwarded to the server (`code_security.rs`, Postgres `cs_*`);
    `datadog-ci sbom upload` goes to the management API (`/api/v2/static-analysis-sca/dependencies`); IAST findings
    are read from the spans' `_dd.iast.json`; advisories come from OSV.dev (`OC_OSV_URL`);
  - events `/api/v1/events`, `/api/v2/events` and the events inside `/intake/`, stored in `otel.otel_events`;
  - host metadata `/intake/` and `/api/v2/host_metadata`, stored in Postgres `datadog_hosts`;
  - live processes and containers `/api/v1/collector` and `/api/v1/container`, stored as `process.*` / `container.*` metrics;
  - network connections and DNS stats `/api/v1/connections`, stored in `otel.otel_network` (Cloud Network Monitoring);
  - orchestrator manifests `/api/v2/orchmanif`, which feed the Kubernetes Explorer;
  - feature flag evaluation over OpenFeature's OFREP (`/ofrep/v1/evaluate/flags[/{key}]`, `oc-intake/src/ofrep.rs`,
    model in `oc-flags`), for any OpenFeature SDK with the OFREP provider; also takes the key as `X-API-Key`;
  - `/api/v1/validate`. The API key is also accepted as `?api_key=`.
- **Management API (server, `server/crates/oc-api/src/routes/datadog_api`):** Datadog's configuration endpoints for
  its Terraform provider and API clients (ADR-0003, `docs/datadog-api.md`), with Datadog's `{"errors": [...]}` shape
  and key headers (`DD-APPLICATION-KEY` / Bearer `ocak_…`, `DD-API-KEY` = ingest key). Contract fixtures from
  `datadog-api-client-go` are downloaded at test time by `make datadog-fixtures`, never vendored (replayed by
  `datadog_api::replay`; `oc-api/tests/datadog_api/README.md`); `make tf-compat`
  runs the real provider. Add a cassette replay test and a `scripts/tf-compat/fixtures/<resource>/` per resource.
- **New features start from the Terraform provider.** Before designing the API of a product area, check whether
  [`DataDog/terraform-provider-datadog`](https://github.com/DataDog/terraform-provider-datadog) (MPL-2.0: read it,
  don't copy it) has resources for it (`datadog_<area>_*`). If it does, the configuration lives under the Datadog
  endpoints those resources call (paths, field names, defaults, read-after-write shape, error codes, as in
  `datadog-api-client-go`), in `datadog_api`, and the UI reads and writes the same records; no parallel
  ObserveCraft-only model for the same thing. Our own `/api/v1/...` routes are only for what the UI needs beyond it
  (aggregations, explorers).
- **Deployment:** use the official Datadog Helm chart and Datadog Operator unchanged; examples are in
  `deploy/examples/datadog`.
- **No agent or SDK of our own:** senders are Datadog's agents and libraries, or OpenTelemetry. The
  `observecraft-agent` chart only configures the official OpenTelemetry Collector image, for OpenTelemetry-only setups.
- **Data model:** keep Datadog metric names as sent, so users' dashboards and monitors keep working. Product pages read
  both naming schemes; `hosts.rs` and `kubernetes.rs` show how.
- **Integrations: [`DataDog/integrations-core`](https://github.com/DataDog/integrations-core)** (BSD-3-Clause) is the
  source for Datadog integrations. We don't need all of them now; take them one by one as the integrations catalog
  grows:
  - The **checks** run inside the Datadog Agent and reach us as ordinary series, so they already work.
  - What to reuse per integration:
    - `metadata.csv` (metric names, types, units, descriptions) for the metric catalog;
    - `assets/service_checks.json`;
    - `assets/dashboards/*.json`, converted to our dashboard format by `scripts/import-integration-dashboards.mjs`
      (which also writes the units from `metadata.csv` that the Agent doesn't send);
    - `assets/monitors/*.json` as recommended monitors;
    - the log pipelines in `assets/logs/`, converted to our pipeline format by `scripts/import-integration-pipelines.mjs`
      (Logs → Configuration → Pipelines installs them from its library);
  - Keep their copyright and LICENSE text with anything copied, and note the source path and version.
- **Still missing:**
  - Cloud Network Monitoring's network paths;
  - the profiler's Thread Timeline and Call Graph views, and Event Management's saved work item states, intelligent (learned) correlation, projects and event pipelines (correlators, maintenance windows and work item attributes exist);
  - Cloud Cost Management's provider bills (`datadog_aws_cur_config`, `datadog_azure_uc_config`, `datadog_gcp_uc_config`),
    tag pipelines and custom allocation rules (budgets, custom forecasts and custom costs files exist, `datadog_api/cost.rs`;
    costs are the cost metrics `all.cost`, `aws.cost.amortized`, …, `cloud_cost.rs`);
  - Cloud Security, the live debugger and remote configuration;
  - Data Streams Monitoring's pathway checkpoints from the tracers (`/api/v0.1/pipeline_stats`, DDSketch in msgpack): its map,
    pipelines and Kafka Console are built from messaging spans and Kafka metrics (`data_streams.rs`, `kafka_console.rs`);
  - RUM (`browser-sdk`, Apache-2.0), which needs a RUM product first.

## Repository layout

| Path | Contents |
|---|---|
| `server/crates/oc-api` | `observecraft-server`: axum routes, auth & tenancy, monitor evaluator, notifier, log archiver, demo seeds |
| `server/crates/oc-intake` | Telemetry gateway on `:4318`: OTLP/HTTP + gRPC, JSON logs, Datadog series; ingest-key auth, per-org rate limits |
| `server/crates/oc-query` | Search syntax → ClickHouse / DuckDB `WHERE` clauses, metric query parser |
| `server/crates/oc-store` | ClickHouse + Postgres clients, migrations, retention |
| `server/crates/oc-tracegen` | Synthetic traces, logs, metrics and CI pipelines |
| `server/migrations/` | ClickHouse (telemetry) and Postgres (metadata) schemas |
| `web/src/api/` | The UI's only data-access layer: typed clients + in-browser mocks |
| `web/src/pages/` | One folder per product area |
| `deploy/` | Docker Compose stack, Helm charts (`deploy/helm/observecraft`, `deploy/helm/observecraft-agent`), Vector and Collector config, examples |

## Commands

```bash
make up         # ClickHouse, Postgres, Kafka, Valkey, Vector, OTel Collector, RustFS, synthetic logs
make server     # API on :8080 (applies migrations)
make intake     # intake on :4318
make tracegen   # demo traces, logs, metrics, CI pipelines
make web        # UI on :5173   (make web-mock: UI against in-browser mocks, no backend)
make test       # cargo test, web typecheck, integration dashboard tests
make openapi    # regenerate the OpenAPI doc and web/src/api/generated/* after API changes
```

Before calling a change done: `make test`, `cargo fmt`, no new `cargo clippy --all-targets` warnings, Prettier on
touched web files (`npx prettier --single-quote --print-width 200 --write <files>`), and for UI changes a look in the
browser (live and, where relevant, mock mode).

## Rules that are easy to break

- **Tenancy.** Organization and user come only from the session or API key, never from request input. Every Postgres
  query filters on `org_id`; ClickHouse telemetry queries filter on `org_id = ?` (`oc.logs`) or
  `OrgId = {oc_org:String}` (`otel.*`) through `AppState::ch(org)`. A test fails on telemetry queries without it.
- **Ingestion goes through Kafka** (docs/adr/0004-kafka-ingestion-buffer.md). The intake writes what it accepts to a
  topic per signal (`oc.otlp.*`, `oc.otlp-json.*`, `oc.logs.json`), keyed by org, with the org's headers
  (`x-oc-org`, retention) as record headers, and answers once Kafka acknowledged (`acks=all`). New telemetry paths
  in the intake go through `send_with` with a `Sink`, never straight to the Collector or Vector; consumers commit
  offsets only after storing. Live tail reads `oc.logs.live` (`live_feed.rs`).
- **SQL.** Bind every user-supplied value as a parameter; never interpolate it.
- **Migrations.** Add a new numbered file; never edit a merged one. ClickHouse DDL must work single-node and clustered:
  use the placeholders documented at the top of `server/migrations/clickhouse/0001_schema.sql`.
- **API.** Document handlers with `utoipa`, keep operation ids unique across route modules, run `make openapi`.
- **Web.** Fetch data only through `web/src/api/*/client.ts` and TanStack Query hooks. Keep the mocks working so
  `make web-mock` shows the feature too. Reuse existing components and CSS classes.
- **Demo data is fictional.** Seeds, generators, mocks, fixtures and screenshots use the e-commerce demo domain
  (`api-gateway`, `storefront-api`, `order-service`, `billing`, `catalog-service`, `inventory-service`, …),
  `example-org/*` repos, hosts like `i-0a1b2c3d4e5f6000N`, AWS account `123456789012`, `@example.com` people.
  Reference screenshots may contain production data: replace every service, team, cluster,
  hostname and ID seen there with neutral ones.
- **S3 storage is RustFS**, never MinIO (compose, archives, tests, docs). Publish its S3 port away from ClickHouse's
  9000 (e.g. `9100:9000`) and use path-style URLs.
- **Commits** follow Conventional Commits (`feat(apm): …`, `fix(monitors): …`); so do PR titles, which become the squashed
  commit that release-please turns into the version and changelog (see "Releases" in `CONTRIBUTING.md`).
- **Dependencies** must pass `cargo deny check licenses` (`server/deny.toml`) and the web license allowlist in `ci.yml`.
- **GitHub Actions** are pinned to a commit SHA with the version in a comment; Dependabot keeps them current.
