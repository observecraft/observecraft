# ObserveCraft: notes for coding agents

ObserveCraft is an open-source, self-hosted, MIT-licensed observability platform modelled on Datadog: logs, metrics,
traces/APM, dashboards, monitors, incidents, infrastructure/containers and CI visibility, built on OpenTelemetry and
ClickHouse. `README.md` covers features and setup; `CONTRIBUTING.md` holds the full coding rules. This file is what an
agent needs on top of them. `AGENTS.md` points here, so keep this the single source of truth.

## Product direction: follow Datadog

ObserveCraft is a Datadog equivalent. When building or changing a feature, Datadog is the spec.

- **Consult the Datadog documentation first** (https://docs.datadoghq.com) for a feature's behaviour, naming, query
  syntax, units, defaults and limits: log indexes, exclusion filters, archives/rehydration, monitor types and
  evaluation, metric query syntax (`avg:system.cpu.user{env:prod} by {host}`), APM concepts, Kubernetes/ECS explorers,
  CI Visibility, DORA metrics, and so on. Use Datadog's terms in the UI and the API.
- **Match the real Datadog UI.** Views should look and behave like the actual Datadog page: same pages, tabs, columns,
  facets, side panels and formats. Don't invent pages, tabs, columns or formats that Datadog doesn't have.
- **Docs text isn't enough for layout.** Fetched doc pages lose their screenshots, and the layout lives in those. Before
  designing or reworking a Datadog-equivalent view, ask the user for screenshots of the real page (they have a Datadog
  account). Use the docs for behaviour and naming, screenshots for layout.
- **Stay compatible with Datadog clients.** Being able to point Datadog tooling at ObserveCraft is a feature. The intake
  accepts the Datadog Agent's and API formats (`server/crates/oc-intake/src/datadog*.rs`, see "Datadog compatibility"
  below). When adding intake endpoints, follow the Datadog API reference
  (https://docs.datadoghq.com/api/latest/) and the public OpenAPI spec in `DataDog/datadog-api-client-*`.
- **OpenTelemetry stays the native path.** Datadog compatibility translates into OTLP and lands in the same ClickHouse
  tables; it never becomes a parallel pipeline.

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
| ClickHouse, DuckDB, NATS, RustFS | Apache-2.0 / MIT |

**Not OK:** Datadog's web app, its design system, icons, logos, illustrations and screenshots are proprietary. Recreate
the layout and behaviour in our own code and assets; never copy their frontend code or images, and don't use the
Datadog name or logo as our branding (naming it to describe compatibility is fine). Don't paste Datadog documentation
text into the repo; write our own. Avoid copyleft or source-available code (GPL, AGPL, SSPL, BSL, Elastic License,
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
  - events `/api/v1/events`, `/api/v2/events` and the events inside `/intake/`, stored in `otel.otel_events`;
  - host metadata `/intake/` and `/api/v2/host_metadata`, stored in Postgres `datadog_hosts`;
  - live processes and containers `/api/v1/collector` and `/api/v1/container`, stored as `process.*` / `container.*` metrics;
  - network connections and DNS stats `/api/v1/connections`, stored in `otel.otel_network` (Cloud Network Monitoring);
  - orchestrator manifests `/api/v2/orchmanif`, which feed the Kubernetes Explorer;
  - `/api/v1/validate`. The API key is also accepted as `?api_key=`.
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
    - `assets/dashboards/*.json`, converted to our dashboard format;
    - `assets/monitors/*.json` as recommended monitors;
    - the log pipelines in `assets/logs/`, once log pipelines exist.
  - Keep their copyright and LICENSE text with anything copied, and note the source path and version.
- **Still missing:**
  - Cloud Network Monitoring's network paths;
  - the profiler's Thread Timeline and Call Graph views, and Event Management's Triage Inbox correlators;
  - Cloud Security, the live debugger, data streams and remote configuration;
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
make up         # ClickHouse, Postgres, NATS, Vector, OTel Collector, RustFS, synthetic logs
make server     # API on :8080 (applies migrations)
make intake     # intake on :4318
make tracegen   # demo traces, logs, metrics, CI pipelines
make web        # UI on :5173   (make web-mock: UI against in-browser mocks, no backend)
make test       # cargo test + web typecheck
make openapi    # regenerate the OpenAPI doc and web/src/api/generated/* after API changes
```

Before calling a change done: `make test`, `cargo fmt`, no new `cargo clippy --all-targets` warnings, Prettier on
touched web files (`npx prettier --single-quote --print-width 200 --write <files>`), and for UI changes a look in the
browser (live and, where relevant, mock mode).

## Rules that are easy to break

- **Tenancy.** Organization and user come only from the session or API key, never from request input. Every Postgres
  query filters on `org_id`; ClickHouse telemetry queries filter on `org_id = ?` (`oc.logs`) or
  `OrgId = {oc_org:String}` (`otel.*`) through `AppState::ch(org)`. A test fails on telemetry queries without it.
- **SQL.** Bind every user-supplied value as a parameter; never interpolate it.
- **Migrations.** Add a new numbered file; never edit a merged one. ClickHouse DDL must work single-node and clustered:
  use the placeholders documented at the top of `server/migrations/clickhouse/0001_schema.sql`.
- **API.** Document handlers with `utoipa`, keep operation ids unique across route modules, run `make openapi`.
- **Web.** Fetch data only through `web/src/api/*/client.ts` and TanStack Query hooks. Keep the mocks working so
  `make web-mock` shows the feature too. Reuse existing components and CSS classes.
- **Demo data is fictional.** Seeds, generators, mocks, fixtures and screenshots use the e-commerce demo domain
  (`api-gateway`, `storefront-api`, `order-service`, `billing`, `catalog-service`, `inventory-service`, …),
  `example-org/*` repos, hosts like `i-0a1b2c3d4e5f6000N`, AWS account `123456789012`, `@example.com` people.
  Reference screenshots may come from a real production Datadog account: replace every service, team, cluster,
  hostname and ID seen there with neutral ones.
- **S3 storage is RustFS**, never MinIO (compose, archives, tests, docs). Publish its S3 port away from ClickHouse's
  9000 (e.g. `9100:9000`) and use path-style URLs.
- **Commits** follow Conventional Commits (`feat(apm): …`, `fix(monitors): …`).
