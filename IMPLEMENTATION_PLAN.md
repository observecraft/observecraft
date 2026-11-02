# Implementation plan: integration details from integrations-core (as Datadog's integration pages)

The catalog tiles come from integrations-core, but the modal shows a generic sentence. Datadog's modal is the
integration's own content: the README (BSD-3-Clause, kept as it is) split into Overview, Configure and Support, the
full `metadata.csv` and the log pipeline's facets under Data Collected, the dashboards and monitor templates under
Monitoring Resources, the CHANGELOG under Release Notes, and the dashboards themselves once data arrives.

## Stage 1: Details importer
Goal: `scripts/import-integration-details.mjs --source <checkout>` writes `web/src/api/integrations/details/<id>.json`
per catalog integration: README sections (Hugo shortcodes resolved, tab markers kept, reference links carried into
each section), resources (Dive Deeper), metrics, log facets, service checks, monitor templates, dashboard titles,
latest CHANGELOG entries.
Success Criteria: node tests on the parsers (sections, shortcodes, metadata.csv quoting, facets) and on the
generated Postgres file (250+ metrics, 5 facets, 2 dashboards, 2 monitors).
Status: Complete

## Stage 2: Modal
Goal: Overview (README overview, categories, Dive Deeper), Configure (the README's setup with Host / Docker /
Kubernetes tabs and copyable code, then our Agent form), Data Collected (log facets table, searchable paged metrics
with units, service checks), Monitoring Resources (dashboards, monitor templates), Support, Release Notes; details
load per integration (lazy JSON chunk).
Success Criteria: node tests on the README segmenter; Postgres in the browser matches Datadog's tabs (mock + live).
Status: Complete

## Stage 5: Dashboards as Datadog draws them
Goal: every pipeline compiles (xml filter, Datadog's tolerance of a stray `|`); ordered dashboards flow in list order;
log widgets of any aggregation and facets (`/api/v1/logs/aggregate`), security signal widgets, geomap, bar chart,
distribution, host map, free text, log patterns; Datadog's metric query and formula forms.
Success Criteria: audits over all 651 dashboards: 0 of 16,970 queries unsupported, 34 of 15,196 formulas (malformed
upstream or outliers/anomalies); Postgres - Overview laid out as Datadog's; conformance tests for Forescout and Mac audit
logs from their integrations-core `_tests.yaml`.
Status: Complete

## Stage 4: Every integration up to date
Goal: the catalog, details, logos, dashboards and log pipelines cover every public integration of integrations-core at
the last revision with all of them (1e7420e, 2026-10-30); the pipeline engine runs what they use (named time zones,
day of year, csv filter, greedyData, Java regex classes, array element filters).
Success Criteria: audit: 391/391 public integrations, 391 details, 651/651 dashboards, 248/248 pipelines; every
pipeline compiles except Forescout (upstream syntax error) and Mac audit logs (xml filter).
Status: Complete

## Stage 3: Dashboards of every integration
Goal: integrations-core dashboards converted for every catalog integration into one compressed server bundle
(`seeds/integrations/core/dashboards.ndjson.gz`), installed for an org once the integration sends data (its metric
prefix seen in the last day; not copied into every org), opened from Monitoring Resources; units of every metric.
Success Criteria: converter tests stay green for all dashboards (no overlap, known widget types); Rust test for the
prefix gating; "Postgres - Overview" renders with our data in the browser.
Status: Complete

---

# Implementation plan: AWS, Azure and Google Cloud overview dashboards

Datadog's AWS Overview, Azure Overview and Google Cloud dashboards aren't in integrations-core (the cloud tiles
live in a private repository), so ObserveCraft builds its own with the same layout: a banner with the cloud's
logo, an introduction with the services' tiles, the cloud's monitors and logs, then one colored group per service
(AWS orange, Azure blue, Google Cloud per service) with KPIs, top lists and time series on the metrics our crawlers
publish (Datadog names). Artwork is ours; logos come from SVG Logos (CC0); services without a CC0 logo get our own
glyph in the provider's palette.

## Stage A: Artwork
Goal: `web/scripts/build-cloud-artwork.mjs` writes the three banners and the service tiles (`web/public/images/integrations/`).
Success Criteria: node test: every file is an SVG with a viewBox and provenance comment; the dashboards' images exist.
Status: Complete

## Stage B: Dashboards
Goal: `scripts/cloud-dashboards.mjs` describes the three dashboards (Datadog widget format, ordered layout) and converts
them with the importer's `convertDashboard` into `server/seeds/integrations/{aws,azure,gcp}/dashboards.json`, installed
with the integration dashboards.
Success Criteria: node tests: metrics belong to our catalogs (AWS statistics suffixes), template variables exist, no
overlapping widgets; the pages render in mock and live mode.
Status: Complete

---

# Implementation plan: Kubernetes integration, Autoscaling, Remediation and Live Rollouts

## Context

- Kubernetes data already arrives: the official Datadog Helm chart / Operator (orchestrator manifests
  `/api/v2/orchmanif`, kubelet and kube-state metrics, live containers) or the OpenTelemetry Collector
  (k8sobjects, kubeletstats). Objects land in `otel.k8s_inventory`; `routes/kubernetes.rs` builds the explorer's
  inventory (`resources()`), with 15-minute usage, requests, limits and owners.
- The Agent's orchestrator check sends Datadog's own CRDs out of the box
  (`orchestrator_explorer.custom_resources.ootb`): `DatadogPodAutoscaler` and `DatadogPodAutoscalerClusterProfile`
  (`datadoghq.com/v1alpha2`) and Karpenter's NodePools. The inventory keeps their whole `spec` and `status`.
- Datadog Kubernetes Autoscaling is done by the **Cluster Agent** (Apache-2.0,
  `pkg/clusteragent/autoscaling/workload`). It applies `DatadogPodAutoscaler` objects (DPAs). DPAs are written by hand
  or generated from a cluster profile for each workload labelled `autoscaling.datadoghq.com/profile: <name>`.
  Recommendations come from:
  - Datadog's backend through Remote Configuration. ObserveCraft doesn't have Remote Configuration.
  - the Cluster Agent's **local fallback** (horizontal, on CPU), used when the backend's values are stale
    (`fallback.horizontal`, enabled by default; built-in and our profiles enable it).
  - an **external recommender**: a DPA annotated `autoscaling.datadoghq.com/custom-recommender:
    {"endpoint": "…"}`. Every 30 s the Cluster Agent POSTs a `WorkloadRecommendationRequest` to that endpoint
    (protojson; agent-payload `autoscaling/kubernetes/recommender.proto`, BSD-3-Clause) and applies
    `targetReplicas` from the `WorkloadRecommendationReply`. ObserveCraft serves this endpoint, so horizontal
    scaling is driven by our metrics.
  - Vertical values (requests) are delivered only through Remote Configuration. That is Datadog's TUF-signed
    service; the `autoscaling.endpoint.url` override must point at a Datadog site. ObserveCraft computes and shows
    the values, and the UI gives the manifest or `kubectl set resources` command to apply them.
  - The Cluster Agent refuses to start autoscaling while Remote Configuration is disabled. The generated values
    therefore enable it. With `site: observecraft.invalid` its requests never reach Datadog.
  - End to end check: minikube with the official chart and Cluster Agent (Stage 3).
- Datadog has no public API or Terraform resource for autoscaling, remediation or rollouts, so these are
  ObserveCraft routes (`/api/v1/kubernetes/...`).

## Decisions

- Costs: per-org prices per vCPU-hour and GiB-hour (Autoscaling → Settings; defaults 0.0316 $/vCPU-h,
  0.0042 $/GiB-h), months of 730 h. Idle = requests − usage; cluster idle = allocatable − requests.
- Recommendations: last 24 h of usage per workload (pod usage summed per owner). CPU request = p95 × 1.15 (≥ 10m),
  memory request = max × 1.15 (≥ 32 MiB). Readiness: Deployment / StatefulSet with requests on every container,
  ≥ 6 h of usage, not already targeted by an autoscaler.
- The external recommender uses the HPA formula on the last 5 minutes:
  `ceil(current × observed / target)`, a 10 % tolerance, and clamping to min/max. Each answer is stored (Postgres
  `kube_autoscaler_recommendations`) so the UI shows that the recommender is connected. The endpoint is on the
  intake (`/api/v2/autoscaling/recommend?api_key=…`, which the Cluster Agent reaches) and is answered by the
  server's internal listener.

## Stage 1: Kubernetes integration page
Goal: Integrations → Kubernetes opens our own page:
- **Clusters**: name, nodes, pods, Kubernetes version, last seen, data sources (orchestrator manifests,
  kubelet metrics, OTel), and whether the Cluster Agent autoscaling CRDs are present.
- **Install**: generates Helm values for the Datadog chart or the observecraft-agent chart, with toggles for
  logs, APM, orchestrator explorer and workload autoscaling.
- **Checks**: the Kubernetes Agent checks from the catalog.
Success Criteria: `GET /api/v1/kubernetes/clusters` tested (tenancy, source detection); the values generator is
unit-tested in node; the page works in mock and live mode.
Status: Complete

## Stage 2: Autoscaling analysis (server)
Goal: `routes/kubernetes_autoscaling.rs`, pure functions over the inventory and 24 h usage:
- workload recommendations, readiness and savings;
- cluster scaling (cluster-autoscaler / Karpenter / none, capacity, idle cost);
- active autoscalers (DPA, HPA) and misconfigured ones (missing target, error condition);
- profiles (cluster profiles seen, plus profiles saved in ObserveCraft);
- settings.
Success Criteria: unit tests for recommendation math, readiness, misconfiguration and cost; route tests for
tenancy; OpenAPI regenerated.
Status: Complete

## Stage 3: External recommender
Goal: The Cluster Agent's custom recommender calls work end to end. The intake admits the key and forwards to
the server; the server computes replicas, answers in the reply shape and records the answer.
Success Criteria: a request built from the agent's protojson shape (fixture written from the proto) returns the
expected `targetReplicas`; a bad key is rejected; the stored answer appears in the autoscaler view.
Status: Complete (verified on minikube with the official chart 7.84.6: the Cluster Agent scaled 1 → 6 → 1 on our replies, source External)

## Stage 4: Autoscaling UI
Goal: Kubernetes → Autoscaling:
- Summary;
- Cluster Scaling;
- Workload Scaling (Scaling Recommendations, Optimized Workloads, Misconfigured Autoscalers, Profiles);
- Create Profile, which builds the `DatadogPodAutoscalerClusterProfile` manifest live;
- the Setup wizard (select workloads → profile / DPA manifest with the recommender annotation → review savings);
- Settings.
Mocks for every view.
Success Criteria: manifest builder unit-tested in node; typecheck; pages checked in mock mode.
Status: Complete

## Stage 5: Remediation and Live Rollouts
Goal:
- **Remediation**: workloads with problems (CrashLoopBackOff, OOMKilled, ImagePullBackOff, unschedulable / pending,
  failing readiness), top clusters, top services, problems by issue type, and our own fix suggestions.
- **Live Rollouts**: Deployments with rollout status (progressing, complete, stalled), availability, progress,
  start and duration (from the newest ReplicaSet and the Progressing condition).
Success Criteria: classification unit-tested; routes tested; pages in mock mode; placeholders removed.
Status: Complete

---

# Previous plan (complete): integrations catalog and AWS end to end

The previous plan (ADR-0003, Datadog-compatible management API) is in git history (`0c363ee`); its Stage 6
(synthetics, logs configuration, metric metadata, span metrics, notification integrations) is still open.

## Context

- The catalog is hand-written mock data (`web/src/api/mock/integrations.ts`); live mode only marks an entry
  "detected" from metric name prefixes. AWS, Azure and GCP do not collect anything: no account model, no crawler.
- `DataDog/integrations-core` (BSD-3-Clause) describes ~400 integrations declaratively: `manifest.json` (tile text,
  `classifier_tags` → categories / OS / data types, `metrics.prefix` and `metrics.check`, `process_signatures`,
  dashboards, monitors), `assets/configuration/spec.yaml` (the check's options: type, required, secret, description,
  example, built from shared templates in `datadog_checks_dev/.../templates/configuration`) and `assets/dataflows.yaml`.
- The cloud crawler tiles (AWS ~100 services, Azure 77, GCP 72) are **not** in integrations-core: they live in a
  private Datadog repository and the crawler is closed source. Public sources: the integration APIs
  (`/api/v2/integration/aws/...`, `/api/v1/integration/azure`, `/api/v2/integration/gcp/...`, cassettes in
  `datadog-api-client-go`), `DataDog/cloudformation-template` and `DataDog/integrations-management` (Azure/GCP
  quickstart scripts), both Apache-2.0. Logos, tile artwork and doc text are ours.

## Crawler placement (decision)

The crawler runs as a background worker in `observecraft-server`, like synthetics and metric rollups:

- Work items in Postgres, one per (account, region, namespace), with `next_run_at`, `claimed_until` and a watermark
  (`collected_until`). Servers claim due items with `FOR UPDATE SKIP LOCKED`, so adding replicas adds crawl capacity
  and an item runs once.
- Points go through Kafka (the metrics topic, `acks=all`) like everything the intake accepts; the watermark advances
  only after Kafka acknowledged, so a crash re-crawls the window instead of losing it, and the stored window is not
  re-sent.
- Per-account concurrency and backoff on AWS throttling; failures (AccessDenied with the missing action, invalid
  role, no data) are stored per account and shown in the tile's Issues tab.
- If crawling ever competes with API traffic, the same binary runs as a separate Deployment with only the workers
  enabled (a Helm value). Not built until needed.

## Stage 1: Catalog generated from integrations-core

Goal: catalog, detail and configuration form driven by one generated file instead of hand-written entries.

Success criteria:
- `scripts/import-integration-catalog.mjs` reads a pinned integrations-core checkout and writes the catalog data
  (tile, categories, OS, data types, metric prefix and check metric, process signatures, config spec with templates
  resolved), with the source version and BSD-3 notice.
- Tests: template resolution in `spec.yaml`, classifier tags mapping, detection from `metrics.check` / prefix.
- The detail view renders the spec as a form and shows the resulting `conf.d/<check>.d/conf.yaml` for the Agent.
- Mock mode and live mode both use the generated catalog; existing dashboard/pipeline imports keep working.

Status: Complete. `scripts/import-integration-catalog.mjs` (pinned revision shared with the dashboard importer)
writes `web/src/api/integrations/catalog.generated.json` (390 integrations), `agentSpecs/<check>.json` (254 specs,
lazy-loaded chunks) and the BSD-3 license; ids are unique (the Agent's core checks share app id `system`), and only
prefixes owned by one integration and outside shared namespaces detect it. `coreLink.ts` links the curated
OTel/Vector entries to their integrations-core entry; the others are listed as Agent checks. `AgentConfigForm`
renders the spec (required first, `anyOf` options as their first scalar form) and the YAML 1.1-safe
`conf.d/<check>.d/conf.yaml`. Tests: `scripts/integration-catalog.test.mjs`. Checked in mock mode; live mode
not run against a backend.

## Stage 2: AWS account configuration API

Goal: `datadog_integration_aws_account` and what the add-account flow needs.

Success criteria:
- Postgres migration for AWS accounts (org-scoped); `/api/v2/integration/aws/accounts` CRUD, `available_namespaces`,
  `logs/services`, `iam_permissions`, `generate_new_external_id`, with Datadog's shapes and errors.
- Cassette replay tests and `scripts/tf-compat/fixtures/aws_account/` passing.
- Our own AWS services catalog (namespace, title, category, metric list) behind `available_namespaces` and the
  AWS sub-tiles.

Status: Complete. `datadog_api/aws_integration.rs` on Postgres `aws_accounts` (migration 0069): accounts CRUD with
`?aws_account_id=`, `available_namespaces`, `logs/services`, `generate_new_external_id`, `iam_permissions`
(`/standard`, `/resource_collection` empty) and Datadog's JSON:API errors. Sections are normalized with the
provider's defaults; an update keeps unsent fields (Datadog's recording shows it), `aws_regions` is replaced whole;
the secret access key is sealed and never answered; provider 4.27.0 reads ids back quoted, which are dropped.
Catalog: `server/seeds/aws_integration.json` (114 namespaces with names, tile ids and categories, 10 Forwarder log
sources; no per-namespace metric lists: the crawler discovers metrics with ListMetrics). IAM permissions are only
what the crawler will call, not Datadog's ~830. Tests: 15 in the module (normalization, tenancy, sealed secret,
replay of `v2/aws_integration` and `v2/aws_logs_integration`; CCM config, EventBridge and the metric name filter
preview not replayed); `scripts/tf-compat/fixtures/aws_account` (account with external id resource, update, the
namespaces and IAM permissions data sources) passes with both auth modes.
Stage 3 note: Datadog's CloudFormation template (`DataDog/cloudformation-template`, Apache-2.0) hard-codes
Datadog's AWS principal and `api.datadoghq.com`; the CloudFormation option needs our own copy of it served by the
server, trusting the server's AWS identity.

## Stage 3: AWS pages

Goal: the AWS tile and add-account flow from the reference screenshots, on our components.

Success criteria:
- Tile page: account list with search and status, per-account tabs (Issues, Account Details, General, Metric
  Collection, Log Collection, Resource Collection), Add AWS Account(s).
- Add account: CloudFormation (template URL + parameters), Terraform (generated HCL for the `DataDog/datadog`
  provider pointed at this install) and Manual (role ARN + external ID); log forwarder autosubscription list from
  `logs/services`; the Forwarder sends to our `/api/v2/logs`.
- AWS service sub-tiles in the catalog; mocks for `make web-mock`; checked in the browser.

Status: Complete. Server: `routes/aws_integration.rs` (session routes `/api/v1/integrations/aws/{setup,external-id,
accounts[/:id]}` over the same rows as the Datadog API; `OC_AWS_PRINCIPAL_ARN` is the server's AWS identity).
Web: `/integrations/aws` (accounts with search, tabs Issues, Account Details, General, Metric Collection,
Log Collection, Resource Collection; AWS services tab; empty state) and `/integrations/aws/add` (CloudFormation:
our own JSON template for the role, downloaded or deployed with the AWS CLI, since console quick-create needs an S3
URL; Terraform: role + account through Datadog's provider against this install; Manually: trust/permissions
policies or access keys; the Datadog Forwarder's own S3 template launched with `DdUrl`/`DdPort`/`DdApiUrl` pointing
here). The generators are pure (`aws/awsSetup.ts`, tests in `scripts/aws-setup.test.mjs`). The catalog's AWS tiles
come from `seeds/aws_integration.json` (the hand-written ones with OTel CloudWatch configs are gone) and are detected
by `aws.<namespace>.` prefixes. Issues show a note until the crawler exists. Checked in mock mode; live mode is
covered by the handler test, not by a browser run; the generated Terraform was not applied against AWS.

## Stage 4: AWS crawler

Goal: metrics from CloudWatch for every configured account.

Success criteria:
- Worker as described above: STS AssumeRole with external ID (server's default AWS credential chain) or the
  account's access keys; ListMetrics + GetMetricData per namespace and region; resource tags joined.
- Datadog metric names and tags (`aws.<namespace>.<metric>`, `region`, `aws_account`, resource ids) so integration
  dashboards and monitors work.
- Tests against a local stub of the AWS endpoints: claiming with two workers, watermark after Kafka ack, throttling
  backoff, AccessDenied surfacing as an account issue, tenancy.

Status: Complete. `src/aws/`: `naming.rs` (Datadog names `aws.<ns>.<metric>` + `.sum` / `.maximum`, lowercased
dimension tags, namespace / metric name / tag filters, dimension → ARN for 11 resource types, OTLP JSON with the EC2
instance as `host.name`), `api.rs` (`AwsApi`; the official AWS SDK: default credential chain, AssumeRole with the
external id or the account's keys, ListMetrics PT3H, GetMetricData 500 per call, Tagging GetResources; errors
classified), `crawler.rs` (jobs per account × region in `aws_crawl_jobs`, migration 0070; `plan` every minute under an
advisory xact lock; `claim` with SKIP LOCKED and a 10-minute lease; Average/Sum/Maximum per minute over the window since
the watermark, ending 5 minutes ago, at most 3 hours; watermark only after every record is acknowledged; backoff by
error kind), `kafka.rs` (rskafka, `oc.otlp-json.metrics`, key = org, the intake's org headers, zstd, acks=all). The
crawler starts with the server when Kafka is configured (now `OC_CLOUD_CRAWLERS`, `OC_CLOUD_CRAWL_*`, Stage 5).
`GET /api/v1/integrations/aws/accounts/{id}/status` feeds the Issues tab and the account list's WARNING badge.
The SDK (Apache-2.0) needed BlueOak-1.0.0 (minicbor) in `deny.toml` and SDK versions for Rust 1.89
(`rust-version` in the workspace); approved by the user. Tests: 26 (pure, crawler with fake AWS and sink, live
Postgres planning/claiming/watermark/status, live Kafka record, live LocalStack for the SDK path with keys and an
assumed role). Not verified: real AWS (no account here) and the collector → ClickHouse leg for these records
(it's the existing `kafka/otlp_json` receiver).

## Stage 5: Azure and GCP

Goal: same pattern (`datadog_integration_azure`, `datadog_integration_gcp_sts`, crawlers for Azure Monitor and
Cloud Monitoring, quickstart scripts from integrations-management pointed at this install). Planned in detail after
Stage 4.

Status: Complete.
- Shared crawler layer `src/cloud/` (`jobs.rs`: `cloud_crawl_jobs` per provider × account × scope, migration 0070
  rewritten before merge; discovery jobs (scope `""`) whose runs become an account's scopes; `kafka.rs`; `naming.rs`;
  `testing.rs`), AWS moved onto it. Config: `OC_CLOUD_CRAWLERS` (`aws,azure,gcp`), `OC_CLOUD_CRAWL_INTERVAL_SECS`,
  `OC_CLOUD_CRAWL_CONCURRENCY`.
- Azure: `datadog_api/azure_integration.rs` (v1 GET/POST upsert/PUT rename/DELETE idempotent, `/host_filters`;
  migration 0071 `azure_accounts`, sealed client secret; replay `v1/azure_integration`; tf-compat `azure_integration`),
  `src/azure/` (`naming.rs`, `api.rs` REST: client-credentials token cache, subscriptions, resources, metric
  definitions, `metrics:getBatch`, tested against a local fake Azure; `crawler.rs`: discovery → subscription jobs,
  batches of 50 resources × 20 metrics, average/total/maximum), `routes/azure_integration.rs`, web
  `/integrations/azure` (+ `/add`: existing app registration via `az ad sp create-for-rbac`, or Terraform), catalog
  tiles from `seeds/azure_integration.json` (55 resource types).
- GCP: `datadog_api/gcp_integration.rs` (`/api/v2/integration/gcp/accounts` CRUD, `sts_delegate` = `OC_GCP_PRINCIPAL_EMAIL`;
  migration 0072 `gcp_accounts`; replay `v2/gcp_integration`; tf-compat `gcp_sts`), `src/gcp/` (`naming.rs` Datadog names
  `gcp.<short>.*`, resource filters, namespaces; `api.rs`: ADC via `gcp_auth` (MIT), impersonation with
  `generateAccessToken`, projects, descriptors, `timeSeries.list` aligned per kind, tested against a local fake
  Google; `crawler.rs`: discovery → project jobs, 8 metric types at a time), `routes/gcp_integration.rs`, web
  `/integrations/gcp` (+ `/add`: Cloud Shell gcloud commands or Terraform), catalog tiles from
  `seeds/gcp_integration.json` (38 services).
- Not verified against the real clouds (no accounts here): Azure and GCP were tested with local fakes of their
  APIs, AWS with LocalStack; Azure's secretless authentication and Datadog's Quickstart scripts aren't supported.
- Not built: resource collection / CSPM (stored only), the CloudWatch metric streams and log forwarding setup for
  Azure (Event Hub) and GCP (Pub/Sub) logs.

## Stage 6: A dashboard per cloud service
Goal: one dashboard per AWS namespace, Azure resource type and Google Cloud service, as Datadog has, installed once
the service's metrics arrive.
Success Criteria: `scripts/cloud-service-dashboards.test.mjs` (metric sources, crawler names, layout, mock subset);
`integration_dashboards` installs `tile:aws-lambda`, `tile:azure-compute-virtualmachines`, `tile:gcp-gce`; service
metrics have units (`metric_unit`); Lambda, Azure VM and GCE dashboards checked in mock mode.
Status: Complete
- `scripts/import-cloud-metrics.mjs`: AWS from aws-cdk-lib's canned metrics (70+ namespaces) plus the AWS
  documentation's CloudWatch metric pages for the rest; Azure from azure-monitor-docs' supported metrics; GCP from
  Cloud Monitoring's metrics list and monitored resources. Names, units, aggregations and dimensions only.
- `scripts/cloud-service-dashboards.mjs`: 204 dashboards (112 AWS, 54 Azure, 38 GCP) in
  `server/seeds/integrations/cloud/services.ndjson.gz`, units in `service-units.csv`.
- Not covered: AWS Budgets and Service Quotas (no metrics of their own; quotas are `AWS/Usage`), Azure Databricks
  (no platform metrics). Some AWS dimensions are inferred where the docs don't name them (noted per namespace, `doc`).

## Stage 7: integrations-extras
Goal: integrations-extras' community Agent checks and log integrations in the catalog, with their details, dashboards,
log pipelines, units and logos.
Success Criteria: catalog, details, dashboards and pipelines tests name extras entries (bind9, unbound, tailscale,
logstash) credited to `DataDog/integrations-extras@EXTRAS_REVISION`; every extras pipeline compiles (grok `rubyhash`
added for Logstash); logos for all but the brands listed in `integration-logos.test.mjs`.
Status: Complete
