# Implementation plan: telemetry storage at SaaS scale

Design: [docs/adr/0002-storage-at-saas-scale.md](docs/adr/0002-storage-at-saas-scale.md). The previous plan
(ADR-0001, metrics storage) is complete; see git history. Each stage is its own PR; `make test` passes at the end
of every stage; single-node and cluster DDL are covered by `oc-store/tests/cluster.rs`.

Baseline (dev, one hour, 2026-10-29): 5k points/s, 90k active series; rows written per hour samples 18.5 M,
series 18.2 M, rollup_5m 18.5 M, rollup_1h 18.5 M; ClickHouse CPU inserts 496 s, merges 601 s, selects 557 s;
Kubernetes dashboard 15–20 s. Every stage is measured against it with a repeatable load: `oc-tracegen` with a
multi-org profile (several orgs, a large one with 100k+ series, pod churn) against local single-node and a
2-shard × 2-replica compose cluster.

## Stage 1: Load profile and measurements

Goal: a repeatable way to produce dev-like (and larger) load and to read the numbers above.

Success criteria:
- `oc-tracegen` profile with N orgs, configurable series per org, label count and pod churn.
- `scripts/storage-report.sql` (or a `make` target) prints rows/bytes written per table, merge and query CPU, and
  the slowest query shapes for a window.
- Baseline numbers recorded here for single node and cluster.
- Cost of decoding and re-encoding OTLP metrics in the intake (CPU per 100k points), to decide whether OTLP
  senders get series marking in Stage 2 or later.

Built:
- `oc-tracegen load` (`server/crates/oc-tracegen/src/load.rs`, `make load`): N orgs, each a fictional Kubernetes
  cluster; the first org has `OC_LOAD_LARGE_SERIES` (100k), the others `OC_LOAD_SMALL_SERIES` (5k); 25 container
  metrics per pod, `OC_LOAD_LABELS` (20) Kubernetes tags, flush every `OC_LOAD_FLUSH_SECS` (15) spread over the
  interval, `OC_LOAD_CHURN_FRACTION` (5 %) of the pods replaced every `OC_LOAD_CHURN_MINUTES` (10).
  `/api/v2/series` JSON (zlib, as the Agent), and `OC_LOAD_OTLP_SHARE` (10 %) of the pods as OTLP/HTTP JSON with
  OpenTelemetry resource names. Deterministic from `OC_LOAD_SEED`; unit tests cover series counts, flush spread,
  label shapes, churn and the OTLP share.
- Orgs and keys: `scripts/load-orgs.sql` (`make load-orgs LOAD_ORGS=4`) creates `load-org-1..N` with the ingest
  keys `ocik_load_org_<n>_local_only`, the profile's default `OC_LOAD_KEYS`.
- `scripts/storage-report.sql` + `scripts/storage-report.mjs` (`make storage-report WINDOW=1h CLUSTER=oc`):
  rows/bytes written per table (`part_log` NewPart, also per hour), merges per table with CPU, CPU by query kind
  plus merges and mutations, top 10 SELECT shapes by CPU. With a cluster name it reads every replica through
  `clusterAllReplicas` (checked against the single-node `default` cluster; a 2-shard run comes with Stage 7).

OTLP decode cost (`otlp_metrics_decode_cost` in `oc-intake/src/otlp.rs`, release, one thread, Apple M4 Max,
the intake's own prost types), per 100k points with 20 labels:
- labels on the points (as the Datadog conversion builds them; 72.6 MiB body): decode 148 ms, series hash
  (sorted labels) 36 ms, re-encode 67 ms, **251 ms**;
- labels on one resource per 25 points (a pod, as OpenTelemetry collectors send them; 7.1 MiB): decode 16 ms,
  hash 43 ms, re-encode 8 ms, **66 ms**.
- At dev's 5k points/s that is 0.3–1.3 % of one core, without gzip. Affordable: OTLP senders can get series
  marking in Stage 2. Caveat: the intake's types hold only the fields the Datadog conversion uses; marking OTLP
  needs the complete metrics messages, or unknown fields (int values, exemplars, histograms, flags) are dropped.

Baseline, single node (2026-10-29, 20 minutes after warm-up, normalized to one hour): ClickHouse 26.8 in Docker
with 2 vCPU / 6 GiB and the chart's tuning and ingest profile (async inserts, 1 s); collector 0.163.2 with
`deploy/otel/collector.yaml`; intake and server from this branch. Load: 4 orgs (80,000 + 3 × 3,300 series),
flush 18 s → 4,994 points/s, 89,900 active series (98,950 seen in 23 minutes with churn), 22 labels per series
after the intake and collector.

| Table | Rows/h (load) | Bytes/h (load) | Rows/h (dev) | Bytes/h (dev) |
|---|---|---|---|---|
| `metrics_samples` | 18.0 M | 297 MiB | 18.5 M | 214 MiB |
| `metrics_series` | 18.0 M | 1.01 GiB | 18.2 M | 911 MiB |
| `metrics_rollup_5m` | 18.0 M | 868 MiB | 18.5 M | 448 MiB |
| `metrics_rollup_1h` | 18.0 M | 868 MiB | 18.5 M | 448 MiB |

| CPU per hour | Load | Dev |
|---|---|---|
| Inserts (Insert 183 s + AsyncInsertFlush 20 s) | 203 s | 496 s |
| Merges (all tables; metric tables 92 s, series alone 46 s) | 127 s | 601 s |
| Selects | 0.3 s (no UI, server background jobs only) | 557 s |

- The write amplification is reproduced: one series row and two rollup rows per point (≈1,440 inserts/h, each
  one part per table).
- CPU is lower than dev: a fresh database merges only small parts (dev's merges also rewrite a day of series and
  rollups), and no one reads. Use the load's numbers, not dev's, as the reference for later stages; for merge
  CPU at steady state run the profile for several hours.
- Not measured yet: the 2-shard × 2-replica compose cluster (Stage 7 builds it), and selects (the profile only writes;
  read load needs dashboards opened against it).

Status: Complete (single node; cluster baseline deferred to Stage 7)

## Stage 2: Write path

Goal: one series row per series and hour; rollups from a background job.

Success criteria:
- Intake: bounded per-replica cache of forwarded series per hour; points of known series go in a request with
  `x-oc-series-known: 1`, which the Collector turns into `oc.series_known = 1`; tests for marking, hour rollover
  and eviction. Datadog-format metrics first; OTLP if Stage 1 shows decoding is affordable.
- Migrations: series views skip marked points; rollup tables become `ReplacingMergeTree` with plain values; the
  per-insert rollup views are dropped; the unused `otel.otel_metrics_*` tables are dropped.
- Rollup job (claimed through Postgres): closed 5m buckets after a lateness window, hours from 5m, reruns
  replace; tests for late points, reruns and gaps; lag exposed as a metric.
- Query layer reads rollups to the last completed bucket and raw samples for the tail; parity tests.
- Under the load profile: series rows/h ≤ 2 × active series; rollup rows/h ≈ series × buckets; merge CPU at
  least 3× lower than baseline.

Status: Not Started

## Stage 3: Tag index and tag values

Goal: filters, group-bys and template variables without scanning label maps.

Success criteria:
- `metrics_tags` and `metrics_tag_values` tables and views (single node and cluster).
- Series resolution through the index; samples read with `MetricName` + `SeriesId IN`; cluster joins shard-local.
- Template variable values and the metric catalog read the small tables.
- Tag values < 100 ms and series resolution of a filtered widget < 100 ms on the load profile.

Status: Not Started

## Stage 4: Dashboard query execution

Goal: per-widget queries, cheaper aggregations, caching.

Success criteria:
- One query per widget in parallel; result cache keyed by org/query/range/step; request coalescing.
- No per-series per-interval `uniqExact`/`medianExactIf`; parity tests with today's results.
- Kubernetes Overview dashboard fully rendered < 2 s on dev-sized load, single node, 4 vCPU.

Status: Not Started

## Stage 5: Kubernetes inventory as columns

Goal: explorers and inventory widgets filter, sort and paginate in SQL.

Success criteria:
- `k8s_resources` typed columns filled at ingest; explorers read columns, `Body` only for details.
- Per-org short cache with coalescing for inventory reads.
- Pods explorer page < 300 ms on dev-sized inventory.

Status: Not Started

## Stage 6: Errors and log facets precomputed

Goal: the Errors page and standard log facets read aggregates.

Success criteria:
- Error groups table fed at ingest; Errors page and facets read it; parity with today's grouping.
- Per-minute aggregate for standard log facets; free-form facets bounded and marked approximate.
- Errors page < 500 ms; facet counts < 300 ms on dev-sized logs.

Status: Not Started

## Stage 7: Cluster and per-org limits

Goal: horizontal scale verified, noisy orgs bounded.

Success criteria:
- 2-shard × 2-replica compose: same results as single node; ingest throughput ~2× one shard under the load
  profile.
- Per-org query limits (time, memory, concurrency) applied per request; tests.
- Dev values: ClickHouse 4 vCPU; dev `otel` database recreated on the new schema.

Status: Not Started
