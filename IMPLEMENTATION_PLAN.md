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

Status: Not Started

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
