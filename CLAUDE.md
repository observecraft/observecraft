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
Datadog name or logo as our branding (naming it to describe compatibility is fine). Datadog's own open-source products
(Apache-2.0 or similar: the Agent and its own checks such as Container, Cluster Agent, Operator, GuardDog, IDE
Shepherd, Supply Chain Firewall, …) are shown under Datadog's name and with Datadog's logo (SVG Logos' `datadog-icon`,
`OVERRIDES` in `integration-logos.mjs`), since the product is theirs; never for our own features or anything proprietary of theirs. Don't paste text from Datadog's
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
  - logs `/api/v2/logs` (its `ddsource`, `service`, `hostname` and `ddtags` query parameters apply to every log, for
    vendors' webhooks, `log_query.rs`);
  - APM `/api/v0.2/traces` (legacy and idx payloads), plus `/api/v0.2/stats`, which is acknowledged;
  - profiles `/api/v2/profile`, forwarded to the server and stored in `otel.profiles` (APM → Profiles);
  - Code Security: the tracers' telemetry `/api/v2/apmtelemetry` (via the Agent; libraries services load) and
    `datadog-ci sarif upload` `/api/v2/cicodescan`, forwarded to the server (`code_security.rs`, Postgres `cs_*`);
    `datadog-ci sbom upload` goes to the management API (`/api/v2/static-analysis-sca/dependencies`); IAST findings
    are read from the spans' `_dd.iast.json`; advisories come from OSV.dev (`OC_OSV_URL`);
  - events `/api/v1/events`, `/api/v2/events` and the events inside `/intake/`, stored in `otel.otel_events`; vendors'
    alert webhooks `/intake/webhook/<name>?api_key=` (SolarWinds, Alertmanager, Zabbix, else generic) become events
    too (`datadog_webhooks.rs`);
  - RUM events of the browser and mobile SDKs `/api/v2/rum` (with the application's client token `pub…`, or through
    the SDK's `proxy` option as `?ddforward=`), stored in `otel.otel_rum` (`datadog_rum.rs`); applications are
    `datadog_rum_application` (`datadog_api/rum.rs`, `routes/rum.rs`); Product Analytics reads the same events
    (`rum_analytics.rs`);
  - Session Replay segments of the browser SDK `/api/v2/replay` (multipart, deflate), stored in `otel.otel_rum_replay`
    (`datadog_replay.rs`); the player, playlists, watch history and heatmaps are `routes/rum_replay.rs`;
  - host metadata `/intake/` and `/api/v2/host_metadata`, stored in Postgres `datadog_hosts`;
  - SBOMs of hosts and container images `/api/v2/sbom` (agent-payload protobuf, `datadog_sbom.rs`), passed to the
    server and stored in Postgres `csm_sbom_entities` / `csm_packages` (Cloud Security, `cloud_security.rs`);
  - live processes and containers `/api/v1/collector` and `/api/v1/container`, stored as `process.*` / `container.*` metrics;
  - network connections and DNS stats `/api/v1/connections`, stored in `otel.otel_network` (Cloud Network Monitoring);
  - orchestrator manifests `/api/v2/orchmanif`, which feed the Kubernetes Explorer;
  - feature flag evaluation over OpenFeature's OFREP (`/ofrep/v1/evaluate/flags[/{key}]`, `oc-intake/src/ofrep.rs`,
    model in `oc-flags`), for any OpenFeature SDK with the OFREP provider; also takes the key as `X-API-Key`;
  - the Agent's runtime security events `/api/v2/secruntime` and `/api/v2/secinfo` (logs payloads,
    `source:runtime-security-agent`), stored as logs (Workload Protection); its inventory `/api/v1/metadata` in
    `datadog_hosts.agent_inventory`;
  - `/api/v1/validate`. The API key is also accepted as `?api_key=`.
- **Management API (server, `server/crates/oc-api/src/routes/datadog_api`):** Datadog's configuration endpoints for
  its Terraform provider and API clients (ADR-0003, `docs/datadog-api.md`), with Datadog's `{"errors": [...]}` shape
  and key headers (`DD-APPLICATION-KEY` / Bearer `ocak_…`, `DD-API-KEY` = ingest key). Contract fixtures from
  `datadog-api-client-go` are downloaded at test time by `make datadog-fixtures`, never vendored (replayed by
  `datadog_api::replay`; `oc-api/tests/datadog_api/README.md`); `make tf-compat`
  runs the real provider. Add a cassette replay test and a `scripts/tf-compat/fixtures/<resource>/` per resource.
- **Cloud SIEM** (`security_*.rs`, `datadog_api/security_monitoring.rs`): detection rules and suppressions are
  `datadog_security_monitoring_rule` / `_suppression` (`/api/v2/security_monitoring/...`); content packs and their
  default rules are our own catalog (`security_catalog.rs`); `security_engine.rs` evaluates `log_detection` threshold
  rules (count, cardinality) every minute into Postgres `security_signals`; the research feed reads CISA's KEV catalog
  (`OC_KEV_URL`).
- **Logs → Configuration** (`datadog_api/logs/`, `logs_config.rs`): indexes (with a flex tier), archives, custom and
  integration pipelines, log-based metrics, restriction queries and custom destinations are `datadog_logs_index` /
  `_index_order`, `_archive` / `_archive_order`, `_custom_pipeline` / `_integration_pipeline` / `_pipeline_order`,
  `_metric`, `_restriction_query` and `_custom_destination` (`/api/v1|v2/logs/config/...`) on the tables the UI edits;
  pipelines store the UI's processor keys and answer Datadog's. Custom destinations are forwarded by the log indexer
  (`log_forwarding.rs`: HTTP, Splunk HEC, Elasticsearch). Archives created through the API need their bucket keys set
  in the UI (Datadog's API carries no secrets).
- **Cloud Cost → Settings** (`datadog_api/cost_bills.rs`, `routes/cloud_cost_bills.rs`, `bills/`): the providers' bills
  are `datadog_aws_cur_config` / `datadog_azure_uc_config` (`/api/v2/cost/aws_cur_config`, `/azure_uc_config`, Postgres
  `cost_bill_configs`), read every 4 hours on the crawlers' jobs with the integrations' credentials (the AWS account's
  role, the Azure app registration): each month's latest Cost and Usage Report (CUR 2.0 or classic) or Azure export run
  (with or without `manifest.json`), loaded with DuckDB and replacing that month's rows of `otel.cloud_costs`.
- **AWS integration** (`datadog_api/aws_integration.rs`): accounts are `datadog_integration_aws_account`
  (`/api/v2/integration/aws/accounts`, Postgres `aws_accounts`, secret access keys sealed), with the external id,
  available namespaces, log sources and IAM permissions endpoints; namespaces and log sources are our catalog
  (`server/seeds/aws_integration.json`). The page (`routes/aws_integration.rs`, `web/src/pages/integrations/aws/`)
  edits the same rows; the server assumes customers' roles as `OC_AWS_PRINCIPAL_ARN`. The CloudWatch crawler
  (`oc-api/src/aws/`, official AWS SDK) runs in the server: jobs per account and region in Postgres `cloud_crawl_jobs`
  (SKIP LOCKED, so servers scale it), points published as OTLP JSON to `oc.otlp-json.metrics` with Datadog names.
- **Azure and GCP integrations** (`datadog_api/azure_integration.rs`, `datadog_api/gcp_integration.rs`; pages
  `routes/{azure,gcp}_integration.rs`): `datadog_integration_azure` (`/api/v1/integration/azure`, Postgres
  `azure_accounts`, sealed client secrets) and `datadog_integration_gcp_sts` (`/api/v2/integration/gcp/accounts`,
  Postgres `gcp_accounts`, impersonated by `OC_GCP_PRINCIPAL_EMAIL`). Their crawlers (`oc-api/src/azure/`,
  `oc-api/src/gcp/`) share the AWS one's jobs and Kafka sink (`oc-api/src/cloud/`): a discovery job per account lists
  its subscriptions or projects, each becoming a job; `OC_CLOUD_CRAWLERS` picks the clouds. Azure metrics follow
  Datadog's names: the bare name is the metric's primary aggregation, `.max` its maximum, `azure.<type>.count` the
  resources (Resource Health `status`); coarse grains (hourly capacity) are published once per series and failed
  reads retried through Postgres `azure_metric_state`; dimensions split per type in `azure/naming.rs` `splits`.
  GCP metrics follow Datadog's too (`gcp/naming.rs` `stats`): `gcp.<domain>.*` (`gce`, `gae`, `gke` aside), cumulative
  counters' total and `.delta`, distributions' `.avg` / `.samplecount` / `.sumsqdev` (`.p95` / `.p99` for the catalog's
  `percentiles`), user labels as tags; late one-minute metrics are read over a window moved back by their ingest delay,
  coarser ones re-read and published once per series and failed reads retried (Postgres `gcp_metric_state`);
  `region_filter_configs` apply.
- **Cloud Cost → Recommendations** (`routes/cloud_cost_recommendations.rs`, `GET /api/v1/cloud-cost/recommendations`):
  our own rules over 14 days of the AWS / Azure crawlers' metrics (idle EC2 instances and Azure VMs, downsizing, RDS
  without connections, idle EBS volumes and managed disks, unused NAT gateways, idle load balancers, unused public IPs);
  costs from the bills' `ResourceId`, else a typical size's list price marked as an estimate. Instance and volume types
  aren't collected, so gp2 → gp3 and type-specific rightsizing aren't there.
- **Kubernetes** (Integrations → Kubernetes, `routes/kubernetes_integration.rs`; Kubernetes → Autoscaling, Remediation,
  Live Rollouts): installed with the official Datadog chart (the Install tab generates its values) or the
  observecraft-agent chart. Autoscaling (`kubernetes_autoscaling.rs`) reads the inventory, the Cluster Agent's
  `DatadogPodAutoscaler` / `DatadogPodAutoscalerClusterProfile` objects and Karpenter NodePools (orchestrator
  manifests), and 24 h of container usage (Datadog `container.*` CPU is in nanocores, marked
  `observecraft.collector = datadog-agent`); prices and saved profiles are Postgres `kube_autoscaling_settings` /
  `_profiles`. The Cluster Agent scales: ObserveCraft serves its external recommender (`custom-recommender`
  annotation → intake `/api/v2/autoscaling/recommend?api_key=` → server `/ingest/autoscaling-recommendation`,
  `kubernetes_recommender.rs`, answers in `kube_autoscaler_recommendations`); it needs Remote Configuration enabled
  (site `observecraft.invalid`) and `DD_AUTOSCALING_WORKLOAD_EXTERNAL_RECOMMENDER_ENABLED`. Remediation
  (`kubernetes_remediation.rs`) and rollouts (`kubernetes_rollouts.rs`) are computed from the inventory.
- **APM → Settings** (`apm_settings.rs`, `datadog_api/apm_config.rs`): retention filters and span-based metrics are
  `datadog_apm_retention_filter` / `_order` and `datadog_spans_metric` (`/api/v2/apm/config/...`, Postgres
  `apm_retention_filters`, `apm_spans_metrics`); span-based metrics are computed every minute from `otel.otel_traces`
  into the metric ingest tables (`run_spans_metrics`); retention filters are evaluated (indexed counts), not applied;
  span pipelines are stored only; ingestion per service reads the tracers' sampling decision (`_dd.p.dm`).
- **App & API Protection** (`appsec.rs`, `datadog_api/appsec.rs`): security traces are the tracers' entry spans with
  `_dd.appsec.json`, login tracking tags or `appsec.blocked` in `otel.otel_traces`; the In-App WAF's custom rules,
  exclusion filters and policies are `datadog_appsec_waf_custom_rule` / `_exclusion_filter`
  (`/api/v2/remote_config/products/asm/waf/...`, Postgres `appsec_waf_objects`); the denylist is `appsec_denylist`;
  API findings are our own rules mapped to the OWASP API Top 10; `application_security` detection rules run in
  `security_engine.rs`.
- **Cloud Security account scans** (ADR-0006, `posture/`, `routes/cloud_posture.rs`,
  `datadog_api/security_findings.rs`): third-party scanners, unmodified and pinned by digest, never floating tags. The
  `scanner` deployment (Prowler's official image + `deploy/helm/observecraft/files/scanner/scan.py`) claims a daily scan
  per AWS account with `cloud_security_posture_management_collection` on from the internal listener
  (`/internal/posture/*`, bearer `OC_SCANNER_TOKEN`), gets credentials narrowed to read-only session policies
  (`SecurityAudit`, `ViewOnlyAccess`, `seeds/posture/aws-scan-session-policy.json`) and uploads OCSF results into
  Postgres `posture_findings`; served as Datadog's `/api/v2/security/findings` and on Cloud Security → Findings.
  Threat detection (CloudTrail) is a separate track (Cloud SIEM).
- **Workload Protection** (`workload_protection.rs`, `workload_catalog.rs`, `datadog_api/csm_threats.rs`): agent
  policies and agent rules are `datadog_csm_threats_policy` / `_agent_rule` (`/api/v2/remote_config/products/cws/...`,
  Postgres `cws_policies` / `cws_agent_rules`; the default ones are our catalog, installed per org); Agents and coverage
  are `/api/v2/csm/onboarding/...` from `datadog_hosts`; events are logs (`source:runtime-security-agent`), monitored
  counts the Agent's `datadog.runtime_security.perf_buffer.events.write`; `workload_security` detection rules run in
  `security_engine.rs`; findings are `workload_activity` rules evaluated over the events. Without Remote Configuration,
  Agents load the downloaded policy files (`/policy/download`).
- **AI Guard** (`ai_guard.rs`, `ai_guard_engine.rs`, `datadog_api/ai_guard.rs`): the tracers' AI Guard SDKs call
  `POST /api/v2/ai-guard/evaluate` on the API (`DD_AI_GUARD_ENDPOINT`, app + API key); the evaluator is our own
  (weighted patterns per attack category, tool-call rules, sensitive data scanners, per-org threshold and per-service
  blocking in Postgres `ai_guard_settings`); the server writes each evaluation to ClickHouse `otel.ai_guard_evaluations`
  itself. The SDK's `ai_guard` spans (client IP, user) are joined from `otel_traces`; Discover reads LLM spans
  (`llm_attrs`); AI Guard's detection rules are `application_security` rules on `@ai_guard.*` queries (tag `ai_guard:true`).
- **Splunk migration** (`spl.rs`, `routes/log_migration.rs`; Logs → Configuration → Import / Translate): SPL is
  translated to our search syntax plus an aggregation (stats/timechart/top/chart), with macros from an uploaded
  macros file; imports of exported alerts and Simple XML dashboards become log monitors and log-stream dashboards.
  Unsupported SPL (eval, rex, join, OR across fields, numeric comparisons) is reported, not guessed.
- **authentik** (Integrations → authentik, `oc-api/src/authentik/`, `routes/authentik_integration.rs`): the server polls an
  instance's events API (`/api/v3/events/events/`, token in Postgres `authentik_connections`, sealed) on the cloud
  crawlers' jobs (provider `authentik`; pages oldest first, from the cursor, so a backlog is read on by the next runs) and
  sends each event as a log (`source:authentik`) to `oc.logs.json`. Saving the connection installs its library pipeline
  (`seeds/pipelines/observecraft.json`, `authentik.category`); its dashboard is ours (`scripts/authentik-dashboard.mjs`
  → `seeds/integrations/authentik/`), listed once connected.
- **NetBird** (Integrations → NetBird, `oc-api/src/netbird/`, `routes/netbird_integration.rs`; Datadog has no NetBird
  integration): the server polls a NetBird Cloud or self-hosted management API (token in Postgres
  `netbird_connections`, sealed) on the cloud crawlers' jobs (provider `netbird`, scopes `events` and `peers`): audit
  events as logs (`source:netbird`) to `oc.logs.json`, peers as `netbird.peer.*` gauges to `oc.otlp-json.metrics`.
  Its dashboard is ours (`scripts/netbird-dashboard.mjs` → `seeds/integrations/netbird/`), listed once connected.
- **FleetDM** (Integrations → FleetDM, `oc-api/src/fleetdm/`, `routes/fleetdm_integration.rs`; Datadog has no Fleet
  integration): the server polls a Fleet server's REST API (`/api/v1/fleet`, an API-only observer's token in Postgres
  `fleetdm_connections`, sealed) on the cloud crawlers' jobs (provider `fleetdm`, scopes `activities` and `hosts`):
  activities as logs (`source:fleetdm`, oldest first from the cursor, so a backlog is read on by the next runs; a
  Fleet ignoring `start_created_at` is read newest first down to the cursor) to `oc.logs.json`; host counts, hosts,
  global policies and the most widespread vulnerabilities as `fleetdm.*` gauges to `oc.otlp-json.metrics` (hosts
  aren't `host`s). Saving the connection installs its library pipeline
  (`fleetdm.category`); its dashboard is ours (`scripts/fleetdm-dashboard.mjs` → `seeds/integrations/fleetdm/`),
  listed once connected.
- **GitLab Audit Events** (Integrations → GitLab Audit Events, `oc-api/src/gitlab_audit/`,
  `routes/gitlab_audit_integration.rs`; Datadog's tile is its own crawler): the server polls GitLab's audit events API
  (Premium/Ultimate; `/audit_events` with an administrator's token, or a group's or project's; token in Postgres
  `gitlab_audit_connections`, sealed) on the cloud crawlers' jobs (provider `gitlab_audit`, scope `events`): GitLab
  answers newest first, so a run walks `created_before` back from the window's end (first pages only, no `Link`
  header or offset pages) and a backlog past a run's pages is carried on by the next runs (`backlog_*_ms`); each event
  is a log (`source:gitlab-audit-events`, `service:gitlab`, `gitlab.*`) to `oc.logs.json`. Saving the connection
  installs its library pipeline (`gitlab.category`); its dashboard is ours (`scripts/gitlab-audit-events-dashboard.mjs`
  → `seeds/integrations/gitlab-audit-events/`), listed once connected; Cloud SIEM pack `gitlab-audit-events`.
- **Personal Settings** (`routes/personal.rs`): per-member preferences (home page, time format, theme, accessibility) in
  Postgres `user_preferences`, email subscriptions (dashboard reports, monitor mentions) and email verification links.
- **Slack** (`routes/slack.rs`, `datadog_api/slack_integration.rs`, page `web/src/pages/integrations/slack/`): workspaces
  are connected through a Slack app (Postgres `slack_accounts`, sealed bot token): "Add to Slack" with this install's app
  (`OC_SLACK_CLIENT_ID` / `_SECRET` / `OC_SLACK_SIGNING_SECRET`) or the bot token of the workspace's own app (manifest on
  the page). Channels are `datadog_integration_slack_channel` (`slack_channels`, Datadog's display options); monitors
  mention `@slack-<account>-<channel>` or `@slack-<channel>`, posted with `chat.postMessage` as a card; mute buttons
  (`/api/v1/public/slack/interactive`, signature-checked) create downtimes as the member with the Slack user's email.
  Other `@slack-<name>` handles are the older incoming-webhook channels (`notification_channels`).
- **Self-monitoring** (`self_monitoring.rs`): one server (advisory lock) writes `observecraft.*` gauges every minute
  straight into `otel.metrics_ingest_gauge` (not through Kafka) of the operator's org (`OC_SELF_MONITORING_ORG`, else
  the first instance admin's): ClickHouse system tables, the log ingester's lag, Postgres, jobs, cost, hourly usage per
  org. Only that org lists the "ObserveCraft Overview" dashboard (`scripts/observecraft-dashboard.mjs`, gated logo
  `observecraft`) and gets the recommended monitors (`observecraft:self-monitoring`); `OC_HEARTBEAT_URL` is the dead
  man's switch outside the install.
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
- **Log normalization is the server's** (`log_normalize.rs`, as Vector's `normalize.vrl` did; no Vector of ours on
  the log path). Multi-line messages are joined only at the source (the agent chart's `node.logs.multiline`
  `recombine`, the Datadog Agent's auto multi-line detection); `log_stack.rs` then reads a joined stack trace into
  `error.stack` / `.kind` / `.message` (Error Tracking, the log side panel), unless the log has them.
- **Integrations: [`DataDog/integrations-core`](https://github.com/DataDog/integrations-core)** (BSD-3-Clause) is the
  source for Datadog integrations. We don't need all of them now; take them one by one as the integrations catalog
  grows:
  - The **checks** run inside the Datadog Agent and reach us as ordinary series, so they already work.
  - The catalog (tiles, categories, detection prefixes, the Agent configuration form) is generated from every
    `manifest.json` and `assets/configuration/spec.yaml` by `scripts/import-integration-catalog.mjs`
    (`web/src/api/integrations/catalog.generated.json`, `agentSpecs/`); curated OTel/Vector entries are linked in
    `coreLink.ts`. Rerun it when the pinned revision changes.
  - Catalog logos (`web/scripts/integration-logos.mjs`, then `npm run logos`): SVG Logos and Simple Icons (CC0) by
    name, the providers' official AWS / Azure / Google Cloud service icons (`fetch-cloud-icons.mjs`, used with the
    owner's approval, terms in `src/assets/cloud-icons/NOTICE.md`) and the other vendors' logos
    (`fetch-brand-icons.mjs`: public-domain Commons files, MIT/CC0 sets, CNCF artwork or the vendor's site icon,
    sources in `src/assets/brand-icons/NOTICE.md`). They are served as files from `public/images/integrations/logos/`
    (manifest `integrationLogos.generated.ts`), so pages load only the logos they show. Datadog's logo only on Datadog's
    open-source products (see the licenses section); never Datadog's other images.
  - Kubernetes kinds (Pod, Deployment, Service, …) show the Kubernetes project's own icons (kubernetes/community,
    Apache-2.0, `web/scripts/fetch-kubernetes-icons.mjs` → `public/images/kubernetes/`) through `KubernetesIcon`; the
    Kubernetes product itself is its logo (`KubernetesLogo`), never a generic ship.
  - `PINNED_REVISION` (`scripts/import-integration-dashboards.mjs`) is the last revision with every integration: from
    2026-10-30 Datadog moves integrations to a private repository, so a newer pin drops catalog entries. Every
    importer above takes the same revision; rerun them all after changing it.
  - [`DataDog/integrations-extras`](https://github.com/DataDog/integrations-extras) (BSD-3-Clause, pinned to
    `EXTRAS_REVISION`, its last revision before the same move) adds the community Agent checks and the integrations
    with log pipelines (113, `repository: 'integrations-extras'` in the catalog); every importer below takes
    `--extras <checkout>` next to `--source`. Its partner and documentation-only listings are left out: they send
    to Datadog itself. Traefik is integrations-core's `traefik_mesh` (moved there from extras).
  - The integration modal is integrations-core's own content: `scripts/import-integration-details.mjs --source
    <checkout>` writes `web/src/api/integrations/details/<id>.json` (README sections with Hugo shortcodes and HTML
    turned into Markdown, Dive Deeper links, metadata.csv, log facets, service checks, monitor templates, CHANGELOG),
    loaded per integration and rendered by `ReadmeMarkdown` (react-markdown, no raw HTML).
  - Every integration's dashboards: `scripts/import-integration-dashboards.mjs --all --source <checkout>` writes
    `server/seeds/integrations/core/{dashboards.ndjson.gz,units.csv}` and the mock subset
    `web/src/api/integrations/coreDashboards.generated.json`; the server installs an integration's dashboards for an
    org when its metric prefix is seen (`routes/integration_dashboards.rs`, from `enabled_integrations`).
  - Integration dashboards draw as Datadog does: an ordered dashboard flows in its list order (`convertDashboard`);
    log widgets read `POST /api/v1/logs/aggregate` (Datadog's `/api/v2/logs/analytics/aggregate` shape:
    count / cardinality / sum / avg / percentiles, any number of facets, totals or timeseries, `log_analytics.rs`),
    security signal widgets the signal search (`signals.ts`); geomaps use Natural Earth countries
    (`web/scripts/build-world-map.mjs`); metric queries accept Datadog's forms (`count:`, `p95:`, no aggregator,
    `.rollup(10)`, inline queries in formulas, `canonicalMetricQuery`).
  - What to reuse per integration:
    - `metadata.csv` (metric names, types, units, descriptions) for the metric catalog;
    - `assets/service_checks.json`;
    - `assets/dashboards/*.json`, converted to our dashboard format by `scripts/import-integration-dashboards.mjs`
      (which also writes the units from `metadata.csv` that the Agent doesn't send);
    - `assets/monitors/*.json` as recommended monitors;
    - every integration's log pipelines in `assets/logs/`, converted by `scripts/import-integration-pipelines.mjs --source
      <checkout>` into `server/seeds/integrations/core/pipelines.json` (Logs → Configuration → Pipelines installs them
      from its library; the mock carries a few in `web/src/api/pipelines/library.generated.json`). The library also
      offers ours for generic formats integrations-core has none for (`server/seeds/pipelines/observecraft.json`, MIT):
      key-value (logfmt) lines, which, as in Datadog, aren't parsed without a pipeline (only JSON is);
  - Keep their copyright and LICENSE text with anything copied, and note the source path and version.
- **Cloud overview dashboards** (AWS, Azure, Google Cloud): Datadog's aren't in integrations-core, so ours are generated
  by `scripts/cloud-dashboards.mjs` (Datadog widget format → the importer's `convertDashboard` →
  `server/seeds/integrations/{aws,azure,gcp}/dashboards.json`, units in `integrations/cloud/units.csv`) on the metric
  names our crawlers publish; banners and service tiles are `web/scripts/build-cloud-artwork.mjs` (our artwork, CC0 SVG
  Logos). Mock data: `web/src/api/mock/cloudMetrics.ts`.
- **Cloud service dashboards** (one per AWS namespace, Azure resource type and Google Cloud service, as Datadog has):
  `scripts/import-cloud-metrics.mjs` gathers each service's metrics into `server/seeds/integrations/cloud/{aws,azure,gcp}-metrics.json`
  (AWS: aws-cdk-lib's canned metrics, Apache-2.0, plus the AWS documentation's metric pages for the rest; Azure: Azure
  Monitor's supported-metrics reference; GCP: Cloud Monitoring's metrics list; names, units and aggregations only), and
  `scripts/cloud-service-dashboards.mjs` turns them into `integrations/cloud/services.ndjson.gz` (installed lazily like
  integrations-core's, by metric prefix), `service-units.csv` and the mock subset `cloudServiceDashboards.generated.json`.
  CloudFront, ECS (+ `ECS/ContainerInsights` → `ecs.containerinsights.*`), RDS (+ Proxy → `aws.rds.proxy.*`) and WAF
  follow Datadog exactly (`scripts/aws-datadog-metrics.json`, merged into `aws-metrics.json` by `import-cloud-metrics.mjs
  --aws-datadog`): the crawler publishes their Datadog names and `aws.ecs.cluster.*` / `.service.*` variants, and
  collects the global services (CloudFront, CloudFront web ACLs) from us-east-1 even when the account's regions leave it
  out (`aws/naming.rs`). In every namespace the bare name carries the metric's statistic (counts are sums), with
  `.sum` / `.maximum` / `.minimum` and the `percentiles` and daily `period`s that file's `overrides` set.
  The most used AWS services (~37: EC2, S3, RDS, the load balancers, Lambda, SQS, …) have a layout designed per service
  in `scripts/aws-service-layouts.mjs` (Datadog's widget format: an Overview group with logo, note, monitor summary
  `tag:(integration:amazon_<svc>) OR aws.<svc>.` and key numbers, then themed groups and the service's logs; our own
  titles and notes); the generator fails on a name the crawler doesn't publish. The most used Azure resource types (~42:
  VMs, AKS, App Service, SQL, Cosmos DB, Storage, Key Vault, Redis, the network services, Event Hubs, Service Bus,
  Backup, OpenAI, …) are designed the same way in `scripts/azure-service-layouts.mjs` (KPIs start with
  `azure.<type>.count` by Resource Health `status`; monitors `tag:(integration:azure_<svc>) OR azure.<type>.`); the
  generator also fails on a `by {tag}` that is neither a resource tag nor a dimension `azure/naming.rs` SPLITS (mirrored
  as `AZURE_SPLITS`; the Rust test `every_azure_dashboard_charts_what_the_crawler_publishes` checks both). The most used
  Google Cloud services (33: Compute Engine, GKE, Cloud Run, Cloud SQL, Pub/Sub, Cloud Storage, Load Balancing, …) are
  designed in `scripts/gcp-service-layouts.mjs` (monitors `tag:(integration:google_<svc>) OR gcp.<svc>.`; cumulative
  counters charted as `.delta`, distributions as `.avg` / `.p95` / `.p99`); the generator fails on an unpublished name, a
  cumulative counter's running total, or a split or filter on a label outside the layout's `labels` (Cloud Monitoring's
  resource and metric labels, which the crawler tags as they are). The other services get the generated
  layout: key numbers, then their metrics grouped as errors / latency / saturation / throughput / capacity.
- **Integration dashboards are listed once their integration is enabled** (`routes/dashboards.rs`, by the dashboard's
  `logo`): AWS / Azure / GCP with an account, synthetics with a test, Kubernetes / hosts / APM with data in the last
  day; other logos always. Cached 30 s per server (`EnabledChange` drops it when an account is added or removed);
  clones (`custom`) aren't affected.
- **Integrations → Install** (`routes/integration_installs.rs`, Postgres `integration_installs`), as Datadog's tiles:
  installing an integration installs its library log pipelines (each names its catalog `integration` in
  `pipelines.json`) and its integrations-core dashboards. Integrations that send only logs (Supply Chain Firewall) have
  no metric prefix to detect, so this is how they get their dashboards. Uninstalling removes the pipelines; the
  dashboards stay. Each catalog entry carries its `collection` (agent, logs, crawler, oauth, webhook, push-site, docs;
  `collectionOf` in `import-integration-catalog.mjs`, README-only cases in its override table): what Datadog's own
  service collects (crawler, oauth) or only documents is "Not available in ObserveCraft" (`web/src/api/integrations/collection.ts`)
  and the server refuses to install it (`seeds/integrations/core/collected-by-datadog.json`, generated with the catalog);
  webhooks get the intake URL to paste, push-site senders only when they take a custom URL (`customEndpoint`).
- **Still missing:**
  - Cloud Network Monitoring's network paths;
  - Logs: forwarding to Microsoft Sentinel (needs Azure credentials), Azure Blob archives, archive readers (roles), the
    pipeline processors the engine doesn't run yet (array, decoder, schema, span id remapper: stored and skipped);
  - the profiler's Thread Timeline and Call Graph views, and Event Management's saved work item states, intelligent (learned) correlation, projects and event pipelines (correlators, maintenance windows and work item attributes exist);
  - Cloud Cost Management's Google Cloud billing export (`datadog_gcp_uc_config`), tag pipelines and custom allocation
    rules (budgets, custom forecasts, custom costs files, `datadog_api/cost.rs`, and the AWS and Azure bills exist; costs
    are the cost metrics `all.cost`, `aws.cost.amortized`, …, `cloud_cost.rs`);
  - Cloud Security's account scans of Azure and GCP (AWS is scanned, see below), identity risks, attack paths,
    compliance framework pages (`datadog_compliance_custom_framework`), image scanning with Trivy and the resource map;
    the live debugger and remote configuration;
  - Workload Protection's delivery of policies to Agents (Remote Configuration; policy files are downloaded instead),
    blocking and kill actions, security profiles and its dashboards (threats, findings, FIM, IMDS, sessions);
  - AI Guard's LLM-based judging (an optional OpenAI-compatible judge is not built), redaction replacements, the
    Supply Chain Firewall verdicts and its Datadog API cassettes (the pinned client commit predates them);
  - App & API Protection's delivery to the tracers (Remote Configuration: WAF rules, blocking, denylist), the managed
    WAF ruleset itself, DAST and its dashboards (attack summary, account takeover, OWASP);
  - Cloud SIEM's other detection methods (new value, anomaly, impossible travel, sequences), signal correlation rules,
    notification rules, threat intelligence enrichment and the investigator; rule case notifications aren't sent yet;
  - Data Streams Monitoring's pathway checkpoints from the tracers (`/api/v0.1/pipeline_stats`, DDSketch in msgpack): its map,
    pipelines and Kafka Console are built from messaging spans and Kafka metrics (`data_streams.rs`, `kafka_console.rs`);
  - APM: applying retention filters and span pipelines to incoming spans (the Collector writes spans directly),
    adaptive sampling and remote Agent ingestion settings (Remote Configuration), service remapping and tag enrichment;
  - Kubernetes autoscaling's vertical values and cluster (Karpenter) recommendations delivered to the Cluster Agent
    (Datadog's signed Remote Configuration; vertical recommendations are shown and applied with the commands the UI
    gives), realized savings against a pre-autoscaling baseline, and pod usage in the Kubernetes Explorer for the
    Datadog Agent's `container.*` metrics (its pods carry no `k8s.pod.uid`);
  - RUM's mobile Session Replay, replay retention and privacy settings, Operations, source maps and debug symbols, generated metrics, retention filters and the
    events search and aggregation API (`/api/v2/rum/events`); the error-focused processing mode keeps every session.

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
make up         # ClickHouse, Postgres, Kafka, Valkey, OTel Collector, RustFS, synthetic logs
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
  offsets only after storing. Live tail reads `oc.logs.live` (`live_feed.rs`). Logs are the server's
  (docs/adr/0005-kafka-log-ingester.md): `log_ingest.rs` reads the log topics, `log_normalize.rs` must stay row for
  row what `deploy/vector/normalize.vrl` makes (change both, regenerate `scripts/log-normalize-golden.mjs`), other
  signals carried as OTLP logs (`oc.signal`) go to `oc.otlp.signals` (`Sink::Signals`) for the Collector.
- **SQL.** Bind every user-supplied value as a parameter; never interpolate it.
- **Migrations.** Add a new numbered file; never edit a merged one. ClickHouse DDL must work single-node and clustered:
  use the placeholders documented at the top of `server/migrations/clickhouse/0001_schema.sql`.
- **API.** Document handlers with `utoipa`, keep operation ids unique across route modules, run `make openapi`.
- **Web.** Fetch data only through `web/src/api/*/client.ts` and TanStack Query hooks. Keep the mocks working so
  `make web-mock` shows the feature too. Reuse existing components and CSS classes.
- **Themes.** Colors come from `web/src/theme/tokens.css` (`var(--…)` in CSS, `token('--…')` in chart options, read at
  render); the dark theme overrides the tokens under `:root[data-theme='dark']`. A color with no token gets a
  `:root[data-theme='dark']` override next to its rule. Check new pages in both themes (profile menu → Theme).
- **Demo data is fictional.** Seeds, generators, mocks, fixtures and screenshots use the e-commerce demo domain
  (`api-gateway`, `storefront-api`, `order-service`, `billing`, `catalog-service`, `inventory-service`, …),
  `example-org/*` repos, hosts like `i-0a1b2c3d4e5f6000N`, AWS account `123456789012`, `@example.com` people.
  Reference screenshots may contain production data: replace every service, team, cluster,
  hostname and ID seen there with neutral ones.
- **S3 storage is RustFS**, never MinIO (compose, archives, tests, docs). Publish its S3 port away from ClickHouse's
  9000 (e.g. `9100:9000`) and use path-style URLs.
- **Commits** follow Conventional Commits (`feat(apm): …`, `fix(monitors): …`); so do PR titles. Prefer rebase merge to
  preserve individual commits while keeping main linear; squash merge remains available when intentionally condensing
  a branch. Release-please reads the commits on main to produce versions and changelogs (see "Releases" in `CONTRIBUTING.md`).
- **Dependencies** must pass `cargo deny check licenses` (`server/deny.toml`) and the web license allowlist in `ci.yml`.
- **GitHub Actions** are pinned to a commit SHA with the version in a comment; Dependabot keeps them current.
