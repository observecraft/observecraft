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
- Cluster: measured in Stage 7 (one against two shards of the compose cluster). Not measured yet: selects (the profile only writes;
  read load needs dashboards opened against it).

Status: Complete (the cluster numbers are in Stage 7)

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

Verified:
Measured locally (2026-10-29): ClickHouse 26.8 in a 4-vCPU container, collector 0.163.2, server + intake
(debug builds) against it; a Datadog `/api/v2/series` load of 91,140 series (Kubernetes Overview metrics with
~20 tags each, plus application metrics; 4,840 names) every 15 s, 6,076 points/s, one org. Baseline = e9bfa94
for 30 min, then the new binaries on the same data (migrations 0009/0010 and Postgres 0047 applied as an
upgrade). Rows and CPU from `system.part_log` / `system.query_log` over 25 min, scaled to an hour.

| Table | Before rows/h | After rows/h | Before bytes/h | After bytes/h |
|---|---|---|---|---|
| `metrics_samples` | 21.9 M | 21.8 M | 329 MiB | 328 MiB |
| `metrics_series` | 21.9 M | 0.09 M steady (0.22 M in the window: intake start + the hour boundary) | 734 MiB | 7 MiB |
| `metrics_rollup_5m` | 21.9 M | 1.1 M steady (2.4 M in the window: 45 min of catch-up) | 963 MiB | 75 MiB |
| `metrics_rollup_1h` | 21.9 M | 0.09 M | 963 MiB | 9 MiB |
| `metrics_tags` (stage 3) | — | 1.5 M per hour boundary (3.5 M in the window incl. the backfill) | — | 19 MiB |

- Series rows/h ≈ 1 × active series; 5m rollups = series × 12; 1h = series. Write amplification ~9× → ~1.4×.
- ClickHouse insert CPU 184 → 45 s/h; merges of the metric tables 47 → 15 s/h (3.1×; all merges incl.
  system tables 164 → 114 s/h). Select CPU 3 → 31 s/h: the rollup job (~0.5 s per 5-minute bucket of this
  org) plus the measurement queries in the window.
- Rollup job: 24 h of empty backfill caught up in two ticks, the 45 min with data in one query (475 ms,
  11.4 M samples read, 547 k rows written); then lag 196–256 s (2 min lateness + the open bucket).
- Tests: intake marking, hour rollover, eviction, org separation, nothing remembered before delivery; the
  wiring marks the second delivery (fake collector); series views skip marked points and samples carry every
  tier (ClickHouse); rollups equal the samples' aggregates, a re-roll after a late point replaces rows
  (FINAL), distributions keep first/last/totals; closed-bucket selection, lateness, catch-up in hour steps;
  Postgres claim (SKIP LOCKED), advance, moved-back progress wins, lag.
- Parity: `new_storage_matches_the_exporter_tables` and `distributions_match_the_exporter_tables` against a
  test-owned copy of the exporter tables (ours are dropped), raw and from rollups with the job's progress at
  everything / partly / nothing rolled up; a mutation of the tail boundary fails both.
- Not done: OTLP passthrough stays unmarked (Stage 1 measured decoding as affordable, but the intake's OTLP
  types are partial: marking it needs lossless splitting).
- Cluster (2 shards + Keeper, ClickHouse 26.8): all migrations apply (`oc-store/tests/cluster.rs`); the rollup
  job rolls each shard's series on that shard (`parallel_distributed_insert_select = 2`) and point queries
  through rollups and the tag index match the oracle (`rollups_and_point_queries_on_a_cluster`).

Status: Complete

## Stage 3: Tag index and tag values

Goal: filters, group-bys and template variables without scanning label maps.

Success criteria:
- `metrics_tags` and `metrics_tag_values` tables and views (single node and cluster).
- Series resolution through the index; samples read with `MetricName` + `SeriesId IN`; cluster joins shard-local.
- Template variable values and the metric catalog read the small tables.
- Tag values < 100 ms and series resolution of a filtered widget < 100 ms on the load profile.

Verified (same setup as Stage 2):
- Index rows carry where a value comes from (Scope bits: point attribute, resource attribute, service, the
  value a plain key reads), so `kube_namespace:x` (resource-only alias), `k8s.namespace.name:x` (point
  attribute wins), `service:x` and missing tags resolve exactly as the label-map expressions:
  `the_tag_index_resolves_series_like_the_label_maps` (15 filter/group shapes incl. shadowed keys,
  wildcards, negations, presence; a mutation of the mapping fails it). All parity tests above run through it.
- Template variable values of the Kubernetes Overview (9 keys): 54–125 ms each, 835 ms together → 4–24 ms
  each, 66–71 ms together. Catalog and tag values parity with the oracle (`direct_readers_match_the_exporter_tables`).
- A filtered widget (`sum:kubernetes.cpu.usage.total{kube_namespace:shop-billing,!kube_container_name:sidecar}
  by {pod_name}`, 1 h): index lookups 8 and 53 ms (16 k and 25 k rows read); the points query reads only the
  matching series (590 k rows, 163 ms).
- Backfill of existing series in migration 0010: 1.46 M index rows from 91 k series.
- Cluster: tags sharded by SeriesId with series and samples, values by (org, key); point queries use
  `distributed_product_mode = 'local'` (without it ClickHouse refuses them: "Double-distributed IN/JOIN
  subqueries is denied", which the previous point queries also hit on a cluster). On 2 shards + Keeper: index
  rows on their series' shard, values reachable by key, point queries equal to the oracle.

Status: Complete

## Stage 4: Dashboard query execution

Goal: per-widget queries, cheaper aggregations, caching.

Success criteria:
- One query per widget in parallel; result cache keyed by org/query/range/step; request coalescing.
- No per-series per-interval `uniqExact`/`medianExactIf`; parity tests with today's results.
- Kubernetes Overview dashboard fully rendered < 2 s on dev-sized load, single node, 4 vCPU.

Verified (same setup; the 53 queries the page sends, from the web's own `collectDashboardQueries`; 37 known
metrics, 16 that the load doesn't send):

| | Before (shared scans) | Stage 3 | Stage 4 |
|---|---|---|---|
| Last hour, first run | 633 ms | 435 ms | 563–577 ms (37 queries, uncached) |
| Last hour, repeated | 577–955 ms | 382–391 ms | 28–30 ms (cached) |
| Last 6 hours, uncached | — | — | 827 ms |
| ClickHouse per run | 4.4 M rows, 1.6 s CPU | 7.8 M rows, 1.4 s CPU | 13.0 M rows, 2.1 s CPU uncached (more rows: each widget resolves its own series); 0 cached |

- Widget results equal the previous shared scan (kept as the test oracle) for gauges, counters, deltas and
  up-down sums, filters, groups and `.weighted()`, raw and from rollups
  (`widget_queries_match_the_shared_scan_on_samples_and_rollups`).
- `.weighted()` counts samples with `count()` (rollups: `sum(Count)`) and takes the mean cadence over the
  group's series instead of `uniqExact` per series and `medianExactIf`: identical for series with one point
  per timestamp and a regular cadence; series with duplicate timestamps or mixed cadences differ (documented
  in the code).
- Cache: `crate::ttl_cache` (shared, swappable for the Valkey cache), entries min(step, 30 s) keyed by org +
  SQL + values, range in whole steps; concurrent identical queries wait for one execution. The live stream
  still sends a snapshot, then deltas.
- Local data is ~1 h old and the series table small, so absolute times are below dev's 15–20 s; dev
  (2 vCPU, 760 k series rows) should be re-measured after deploying.

Status: Complete (single node)

## Stage 5: Kubernetes inventory as columns

Goal: explorers and inventory widgets filter, sort and paginate in SQL.

Success criteria:
- `k8s_resources` typed columns filled at ingest; explorers read columns, `Body` only for details.
- Per-org short cache with coalescing for inventory reads.
- Pods explorer page < 300 ms on dev-sized inventory.

Done: `otel.k8s_inventory` (migration 0011, `ReplacingMergeTree(LastSeen)` by `(OrgId, Kind, Uid)`, sharded
like the objects) is filled by a view from `otel_k8s_objects`: typed metadata (namespace, owner, created,
deletion, labels, annotations), pod columns (phase, node, IP, host network, ready / total containers,
restarts, CPU / memory requests and limits) and `Doc`, a trimmed JSON of the spec and status fields the
explorer's rows are built from. The explorer reads it (`FINAL` on these columns only), the detail panel reads
the full object from `k8s_resources` by uid, and Network's pod / service address lookups read the typed
columns. Rows depend on each other (owner chains, rollups, clusters / containers / images derived from pods)
and on usage metrics (CPU sorts), so the built inventory is kept per org for 15 s with request coalescing
(`ttl_cache.rs`) and explorers, widgets, the overview, catalog, SQL editor and host panel filter, facet and
sort it in memory. No backfill: collectors resend every object within minutes.

Measured (local single node, ClickHouse 26.8 limited to 2 vCPU, 9,363 objects in one cluster, 80 MiB of
bodies, release server; medians of 5):

| | before | after, uncached | after, cached |
|---|---|---|---|
| inventory query | 290 ms, 204 MiB read, 79 MiB sent | 21 ms, 15.5 MiB read, 8 MiB sent | — |
| pods explorer page | 571–776 ms | 173–181 ms | 17 ms |
| orchestration search (widget, sort by CPU) | 606–706 ms | 153–159 ms | 18 ms |
| six widgets at once | 1.3–1.6 s | 137–173 ms | 37 ms |
| pod detail | 545 ms | — | 7 ms |

Parity: a ClickHouse test (`OC_TEST_CLICKHOUSE_URL`) writes a fixture of every kind and pod state through
the view and checks that each built row equals the row of the full object, and the typed pod columns equal
the rows' values. Old and new servers on the same 9.4k-object data: 240 responses (every kind, 24 sorts, 24
filters, group-bys, per-namespace sets, 60+ details) are identical apart from the order of rows whose sort
keys tie, which the old `FINAL` read left undefined (two old responses differ the same way); the new read
orders by cluster, kind, namespace, name.

Status: Complete

## Stage 6: Errors and log facets precomputed

Goal: the Errors page and standard log facets read aggregates.

Success criteria:
- Error groups table fed at ingest; Errors page and facets read it; parity with today's grouping.
- Per-minute aggregate for standard log facets; free-form facets bounded and marked approximate.
- Errors page < 500 ms; facet counts < 300 ms on dev-sized logs.

Status: Complete

Built: ClickHouse migration `0012_log_aggregates` adds `oc.error_groups_1m` (AggregatingMergeTree per org, minute,
service, error_kind, error_key, index, host, source, env, version, kube_namespace: count, first/last seen, argMax
sample) and `oc.log_facets_1m` (SummingMergeTree per org, minute and the default facets: index, service, status,
host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name), both fed by views
over the shard-local `oc.logs`, partitioned and expired by the logs' `retention_days`. `routes/log_aggregates.rs`
plans each read: whole minutes from the org's first complete aggregate minute on come from the aggregate, partial
edge minutes and older logs from `oc.logs`, so results equal a read of the logs. Searches or restriction queries
on anything else (free text, other tags, attributes, rehydrated logs) read the logs only. Facets the aggregate
doesn't keep (pod_name, attributes, added facets), and every facet of such searches, scan at most ~1M of the
newest logs of the range (from the aggregate's per-minute totals), scale the counts and return
`approximate: true`, shown as "≈" in the facet panel (mocks too).

Measured (local ClickHouse 26.8 limited to 2 vCPU / 6 GiB, synthetic dev-sized logs: 6.5M rows/h, 4% errors,
40 services, 14 hosts; median of 5, ClickHouse CPU per call):

| Read | Range | Before (raw logs) | After |
|---|---|---|---|
| Error groups (issues search) | 1 h / 2 h | 92 ms, 0.19 CPU-s / 84 ms, 0.21 CPU-s | 24 ms, 0.03 CPU-s / 27 ms, 0.03 CPU-s |
| Errors page facets | 1 h / 2 h | 183 ms, 0.38 CPU-s / 191 ms, 0.43 CPU-s | 24 ms, 0.03 CPU-s / 25 ms, 0.03 CPU-s |
| Log facets (all 14) | 1 h / 2 h | 2.26 s, 4.4 CPU-s / 3.37 s, 6.6 CPU-s | 174 ms, 0.33 CPU-s / 181 ms, 0.35 CPU-s |

Before = the pre-stage SQL run directly; the endpoints forced onto the logs path (a no-op text term) give the
same order (error groups 73–112 ms, Errors facets 99–265 ms). Of the log facets' 180 ms, the standard facets'
query is ~110 ms (mostly the partial edge minutes read from the logs: ~2 granules per service) and runs in
parallel with the bounded scan of pod_name and attributes (~130 ms). Error reads stay flat with the range; the
standard facets' aggregate part grows with it (~565 rows per minute here). Parity: live tests compare both
aggregate-backed queries with today's SQL on fixtures (edges, pre-aggregate logs, filters, restrictions).

## Stage 7: Cluster and per-org limits

Goal: horizontal scale verified, noisy orgs bounded.

Success criteria:
- 2-shard × 2-replica compose: same results as single node; ingest throughput ~2× one shard under the load
  profile.
- Per-org query limits (time, memory, concurrency) applied per request; tests.
- Dev values: ClickHouse 4 vCPU; dev `otel` database recreated on the new schema.

Built:
- `deploy/compose/clickhouse-cluster.yml` (`make cluster-up` / `cluster-down`): ClickHouse 26.8, cluster `oc` of
  2 shards × 2 replicas (`{shard}` / `{replica}` macros, `internal_replication`, the chart's tuning and users, 2 vCPU
  and 6 GiB per node) on `:18161`–`:18164`, plus `oc_1shard` (the first shard's two replicas) to compare; one Keeper
  (placement, replication and distributed queries are what is checked, not Keeper's fault tolerance; production uses
  3). The `pipeline` profile adds the collector, inserting round-robin over the nodes. CONTRIBUTING.md has the usage.
- Parity: `OC_TEST_CLICKHOUSE_URL=http://localhost:18161 OC_TEST_CLICKHOUSE_CLUSTER=oc cargo test` runs every live
  test on the cluster (`test_support::live_clickhouse`): rollups against the samples, query parity raw and from
  rollups, the tag index, direct readers, dashboard widgets against the shared-scan oracle, error groups and log
  facets against the logs, the Kubernetes inventory against the full objects, the shared caches across two server
  replicas; 803 tests pass on the cluster and on a single server. `tests/cluster.rs` runs repeatably (its own orgs),
  also checks the series → samples join and `SeriesId IN` through the server's client, the latest Kubernetes
  snapshot winning in `FINAL`, and that after `SYSTEM SYNC REPLICA` every replica of a shard holds the same rows of
  every table, materialized views' targets included.
- Cluster-only bugs fixed: (1) joins and `IN` subqueries of Distributed tables inside queries of Distributed tables
  (`points_subquery` of hosts, pods, processes, fleet, watchdog, databases) failed with "Double-distributed IN/JOIN
  subqueries is denied": the server's client sets `distributed_product_mode = 'global'` on a cluster
  (`oc_store::clickhouse`), `points_client` keeps `local` for the co-located series tables; (2) the rollup job's
  shard parts read whichever replica runs them (the local one, whatever its delay), so a lagging replica would roll
  a bucket up from incomplete samples once and for all: the job waits while a replica of the metric tables lags more
  than half the lateness window (`system.replicas.absolute_delay` of every replica). No issue found with `FINAL`
  (every `FINAL` read is on a table whose rows of a key share a shard), MVs on `_local` tables, the rollup
  `INSERT … SELECT` (`parallel_distributed_insert_select = 2`) or ReplacingMergeTree dedup across replicas. A crawl of
  the API's 122 parameterless GET and read POST endpoints on the cluster with demo data: no 5xx, no ClickHouse
  exception in `query_log` (92 query shapes).
- Per-org query limits (`org_limits.rs`): every query of an org carries `max_execution_time`, `max_memory_usage` and
  optionally `max_rows_to_read` (+ `read_overflow_mode = 'throw'`), `OC_ORG_QUERY_*` (Helm `query.*`); at most
  `OC_ORG_QUERY_CONCURRENCY` (16) queries of one org run at once per server replica, more wait
  `OC_ORG_QUERY_QUEUE_TIMEOUT_MS` (10 s) and then get 429 + `Retry-After: 1`. The limit sits in the HTTP connector of
  each org's connection pool, so every `AppState::ch` call site (cache loads included) is limited where the query
  runs. Background jobs (`org_limits::background`: rollups, monitors, schedulers, archiver) are exempt from the
  concurrency limit, not from time and memory. Queries ClickHouse stops at a limit answer 503 (time, memory, rows,
  server busy) or 429 (quota) with what to change instead of 502. Tests: settings parsing, the settings on a live
  query, concurrency and queueing, one org not holding up another, the 429, the background exemption, the error
  mapping.

Measured (2026-10-29, Apple M4 Max with 16 cores for everything; Docker; ClickHouse nodes capped at 2 vCPU / 6 GiB;
release intake and server, collector 0.163.2; the Stage 1 profile from 4 `oc-tracegen load` processes, 8 orgs, 20
labels, flush 10 s, 10 % OTLP; a fresh cluster and database per topology; rates from `part_log`, CPU from
`make storage-report CLUSTER=…` per server):

| | one shard (`oc_1shard`, 2 replicas) | two shards (`oc`, 2 × 2 replicas) |
|---|---|---|
| 202k points/s offered | sustained: 202k landed every minute for 9 min, data 1–2 s old | sustained: 190–209k per minute, 1–4 s old |
| CPU per node at 202k/s | 4,560–4,900 CPU-s/h (1.3 cores of 2; merges peak at the cap) | 3,250–3,570 CPU-s/h (0.9–1.0 cores) |
| more offered | 252k: 160–190k landed, data 43 → 183 s old (backlog) | 302k: ~190k landed, data ≤ 13 s old: the host was the limit (load average 18.5 on 16 cores), not ClickHouse |
| rows/h at 202k/s | samples 0.72 G, tags 1.67 G, series 73 M, tag values 53 M | samples 0.71 G, tags 1.63 G, series 71 M, tag values 62 M |
| merges (all nodes) / inserts | 5,142 / 4,335 CPU-s/h | 6,676 / 6,719 CPU-s/h |
| rollup job | lag 15–390 s (lateness 120 s + open bucket), 5m INSERTs 117 ms mean | same; never waited for a replica |

- One shard's ceiling is ≈ 200k points/s. Two shards carry the same load at ~70 % of the per-node CPU, so by CPU per
  point their ceiling is ≈ 275k/s, **≈ 1.4×, not the 2× targeted**; a direct measurement needs the load generated on
  other hardware than the cluster's.
- Why less than 2×: rows cost more on two shards. Each node's Distributed insert splits into one part per shard, so
  each shard gets twice the parts (samples: 720 against 335 new parts a minute), and merges (+30 %) and inserts
  (+55 %) of the cluster grow per row. Next: batch the Distributed sends (`distributed_background_insert_batch`),
  or send each shard's rows to its own nodes, and larger async insert buffers.
- The tag index dominates ingest: the 10 % OTLP points are not marked (Stage 2), so each is a series row with 20 tag
  index rows (1.6 G rows/h, 56 % of merge CPU on one shard). Marking OTLP in the intake is the largest saving left.
- Limits on the cluster (one org with 750k series, heavy grouped queries of 1.4–3.4 s): concurrency 2 with a 1.5 s
  queue, 12 at once: 2 ran, 10 got 429 after 1.5 s while a quiet org's query still ran (2.4 s, 1.4 s alone);
  concurrency 4 with the 10 s queue: all 12 queued and completed (8.6–9 s), the quiet org's in 4.3 s.
- Not done: the dev values live outside this repository; the Helm chart states at least 4 vCPU per ClickHouse
  server and that dev should move from 2 to 4, and the dev `otel` database is still to be recreated there.

Status: Complete in this repository (cluster, parity, limits, sizing docs); ingest scaling measured at ≈ 1.4× per
added shard, below the 2× target (see above); dev values pending outside the repository

## Stage 8: Shared cache

Goal: one cache that every `observecraft-server` replica shares (ADR-0002, "Shared cache: Valkey, required"), so
results computed by one replica serve the others.

Success criteria:
- `OC_CACHE_URL` required: the server exits at startup with a clear message when it is missing, not a
  `redis://`/`rediss://` URL, or unreachable for 30 s; the intake doesn't need it.
- `oc-api/src/cache.rs`: `get_or_load(namespace, org, key, ttl, load)` with org-scoped, hashed keys; compact values
  with a size cap; coalescing within a replica and across replicas (lock, wait, take over from a dead holder);
  `invalidate(namespace, org)` by generation; counters on the internal listener; requests served from the source
  when Valkey fails. Unit tests on a fake backend, live tests (`OC_TEST_VALKEY_URL`) with two caches as replicas.
- `ttl_cache.rs` bounded (entries, total weight, LRU); the inventory cache bounded by objects; the portal's
  duplicate cache folded onto it.
- Compose `valkey` service; Helm: bundled Valkey (no persistence, maxmemory from the memory limit, LRU,
  NetworkPolicy, non-root) or `cache.mode=external`; `helm lint` (also `values-ha.yaml`) and `helm template` pass.

Built: `cache.rs` on the `redis` crate (BSD-3-Clause; async `ConnectionManager` reconnecting in the background, 1 s
response timeout, TLS over rustls/ring). Keys `oc:{ns}:{org}:{generation}:{sha256(key)[..16]}`; the org segment is
percent-encoded outside `[A-Za-z0-9._-]`. A missing generation key (evicted) gets a new random one, so old entries
can't come back. Values: one format byte, then zstd level 1 over MessagePack (`rmp-serde`, named fields: as
tolerant of `serde` attributes as JSON); above `OC_CACHE_MAX_VALUE_BYTES` (8 MiB) a one-byte marker is stored so
other replicas load at once instead of waiting for a lock. Lock: `SET …:lock <token> NX PX <load timeout>`,
released by a compare-and-delete script (also, in the background, when the holder's request is cancelled
mid-load, so other replicas don't wait out the lock); waiters poll from 5 ms doubling to 100 ms, take the lock over when it is
gone, and load without it after `OC_CACHE_LOAD_TIMEOUT_SECONDS` (30). After a Valkey error the cache is skipped
for 1 s; warnings at most every 30 s. Counters (`oc_cache_lookups_total{result}`, `_loads_total`,
`_coalesced_total{scope=replica|cluster}`, `_lock_timeouts_total`, `_oversized_total`, `_errors_total`,
`oc_cache_value_bytes` histogram, per namespace) at `GET /metrics` on the internal listener (`OC_LOGPROC_ADDR`,
behind `OC_INTERNAL_TOKEN` when set). `ttl_cache.rs`: `with_max_entries`, `with_max_weight(max, weigher)`, LRU over
finished entries, values heavier than the bound returned but not kept, plus `get` / `insert` / `remove_prefix`;
the inventory is bounded by `OC_INVENTORY_CACHE_MAX_OBJECTS` (500k), the portal home's answers by 2,000 entries.

Measured (release, Apple M4 Max, Valkey 8.1 in Docker on the same machine):

| Value (as JSON) | Stored | Encode | Decode |
|---|---|---|---|
| 1.0 MiB of metric series (13 tags, 300 points, noisy values) | 188 KiB (17.8 %) | 1.7 ms | 1.6 ms |
| 10.3 MiB of the same | 1.7 MiB (16.2 %) | 14.1 ms | 10.5 ms |

MessagePack against JSON (both zstd 1, repetitive series): 1 MiB → 34 KB vs 79 KB, write 1.4 ms vs 2.5 ms, read
1.1 ms vs 2.2 ms; 10 MiB → 283 KB vs 738 KB, write 13 ms vs 18 ms, read 11 ms vs 21 ms. LZ4 was faster to
decompress but stored 4–8× more. A hit (generation + value) costs 0.24 ms for 1 KiB and 0.48 ms for 100 KiB:
no in-process layer in front of Valkey. Coalescing (live tests): three concurrent misses on two caches run one
load (one waiter shares the replica's load, one waits for the other replica's); a lock left by a dead replica
delays the next load by its lock TTL only.

Call sites (after Stages 2–4), each namespace scoped by org:
- `widgets`: dashboard widget rows (`metrics/dashboard.rs`, `widget_rows`), keyed by SQL + bound values (step-rounded
  range, rollup progress), TTL min(step, 30 s). The in-process `results_cache` is gone; the cache module coalesces
  concurrent identical queries on a replica. Dashboard edits need no invalidation: a changed query is another key.
- `tag_values` (`/api/v1/tags/{key}/values`) and `metric_catalog` (`/api/v1/metrics`), 60 s. The catalog lists
  derived metrics, so derived metric writes invalidate `metric_catalog`.
- `metric_index_rules`: the indexing controls (`metric_rules.rs`) moved off a per-process map, 30 s; rule writes
  invalidate them, so a rule written on one replica applies to the next query on all. Widget results need no
  invalidation on rule changes: the allowlist is checked before the cache, and ingestion controls only affect
  points stored later.
- `kube_inventory`: the bounded in-process `ttl_cache` stays the first layer (15 s); its load goes through the
  shared cache (`shared_inventory`), so one replica builds and the others decode its copy. A decoded copy can be
  up to two TTLs (30 s) old; collectors refresh objects every minute.

Inventory size: 137 copies of the test fixture (10,001 objects, 13,306 rows, 25.5 MB as JSON) store as 134 KB, as
the copies repeat each other; at one copy's compression ratio (0.071) unrepeated objects would take ~1.8 MB, well
under the 8 MiB cap (test `a_dev_sized_inventory_fits_in_the_shared_cache`; release: encode 17 ms, encode +
decode 50 ms). The same 10,001 objects written through `otel_k8s_objects` stored 283 KB per build.

Measured (release, two `observecraft-server` processes on one Valkey 8.1, ClickHouse 26.8 in Docker; org with 48
Kubernetes metrics × 600 series × 1 h at 20 s; the Kubernetes - Overview dashboard's 51 queries in one
`/api/v1/query/dashboard`, three rounds):

| | Replica A, first | Replica B, after A | B again | B alone (entries expired) |
|---|---|---|---|---|
| Kubernetes - Overview (51 queries) | 227–251 ms, 50 scans | 13–14 ms, 0 scans | 11 ms | 206 ms, 50 scans |
| Kubernetes Explorer, pods (10k-object inventory) | 136–152 ms (build) | 46–55 ms (decode) | 12–14 ms (memory) | 142 ms (build) |
| Metric catalog / tag values | 75 ms / 10 ms | 1.9 ms / 2.0 ms | | |

`/metrics` on both internal listeners showed the split per namespace, e.g. replica B: `widgets` 440 hits / 50
misses, `kube_inventory` 3 / 1, `metric_catalog` 1 / 0, `tag_values` 1 / 0; replica A did the loads.

Tests: fake-backend unit tests (two replicas share one widget execution per org and per bound values; one
inventory build decoded by the other replica, which then serves it from memory; widget TTL), live tests on two
`AppState`s sharing one fake Valkey (`test_support::replicas`, `OC_TEST_CLICKHOUSE_URL` + `OC_TEST_POSTGRES_URL`):
a dashboard answered by B from A's execution, tag values and catalog shared per org with derived metric
invalidation, indexing rules written on A applied on B.

Not switched (per replica, listed for later): SLO measures (`slos.rs`, dropped only on the writing server), the
derived metric map (`derived_metrics.rs`, 30 s, read several times per query), the metric kind / distribution /
retention probes in `metrics.rs` (tiny, per process), and the portal home's provider answers (per user). Open:
`/metrics` reachable by a scraper (NetworkPolicy) when we monitor ObserveCraft with itself.

Status: Complete

## Final test: before/after

Old = 83d6fe6 (release 0.3.2, what dev runs), new = 2f753e9 (this branch), measured 2026-10-29 side by side on one
Apple M4 Max (16 cores, shared with other local workloads: a k3s cluster and other containers), fed identical
metrics and Kubernetes objects. Scripts, ports and the exact request list: `scripts/final-test/README.md`.

Setup, per version, isolated (own Docker network, containers `ocfin-old-*` / `ocfin-new-*`):
- ClickHouse 26.8, `--cpus 2 --memory 6g`, the chart's tuning (mark caches 512 MiB, background pool 4, 100
  queries) and ingest profile (async inserts, 1 s); that version's collector 0.163.2 config, its Vector config + NATS;
  release intake and server of that version; Postgres DB `oc_final_old` / `oc_final_new`; new: Valkey 8.1 (`OC_CACHE_URL`).
- Orgs and keys from `scripts/load-orgs.sql` (`load-org-1..4`), plus the catch-all log index that org provisioning
  creates and a local test user in `load-org-1` (the server installs the integration dashboards for orgs with users).
- Load, the same to both (new's `oc-tracegen`, one process per intake): the Stage 1 profile, seed 1, 4 orgs
  (80,000 + 3 × 3,300 series), 20 labels, flush 18 s (4,994 points/s, 89,900 active series), churn 5 % / 10 min,
  10 % OTLP; `oc-tracegen` demo traces at 200 traces/s into `load-org-1` (6.5 M spans/h and 2.6 M logs/h, 4 %
  errors, plus its host, Kubernetes, Lambda, Kafka and application metrics, CI runs, events, profiles, network);
  `scripts/final-test/k8s-objects.mjs`: 9,566 Kubernetes objects of one cluster (`shop-prod-eks`: 107 nodes,
  3,714 pods — the load profile's 3,200 pods of org 1 with their churn, plus DaemonSet and Job pods — 400
  deployments, 1,200 replica sets, services, HPAs, PVCs/PVs, CronJobs/Jobs, RBAC, …), built from the test
  fixture's objects with fictional names and sent through the intake like the Cluster Agent's manifests (OTLP logs
  marked `oc.signal = k8s.objects`), each object once a minute, a sixth every 10 s, churned pods at once.
  `oc-tracegen`'s traces seed from the clock, so the stacks got statistically equal, not identical, spans and logs.
- Warm-up 52 minutes (load from 13:53 UTC), then measurements alternating old and new (odd runs old first, even
  runs new first).

### Writes and ClickHouse CPU per hour

From `system.part_log` / `system.query_log` over 14:05–14:45 UTC (40 minutes after warm-up, before the page
measurements; `scripts/final-test/writes-window.sql`, the measures of `make storage-report`), scaled to an hour;
new's `metrics_rollup_1h` from `make storage-report WINDOW=1h` (written at the hour boundary).

| Table | Old rows/h | Old MiB/h | New rows/h | New MiB/h |
|---|---|---|---|---|
| `metrics_samples` | 18.61 M | 305 | 18.61 M | 307 |
| `metrics_series` | 18.40 M | 1,056 | 2.28 M | 100 |
| `metrics_rollup_5m` | 18.39 M | 880 | 1.10 M | 52 |
| `metrics_rollup_1h` | 18.39 M | 880 | 0.09 M | 4 |
| `metrics_tags` | — | — | 47.74 M | 242 |
| `metrics_tag_values` | — | — | 1.55 M | 22 |
| **metric tables** | **73.8 M** | **3,121** | **71.4 M** | **728** |
| `otel_k8s_objects` / `k8s_resources` | 0.57 M / 0.57 M | 58 / 80 | 0.57 M / 0.57 M | 58 / 80 |
| `k8s_inventory` | — | — | 0.57 M | 75 |
| `oc.logs` / `logs_raw` | 2.62 M / 2.62 M | 168 / 169 | 2.63 M / 2.63 M | 169 / 169 |
| `log_facets_1m` / `error_groups_1m` | — | — | 12 k / 9 k | 3 / 4 |
| `otel_traces` | 6.46 M | 237 | 6.49 M | 237 |

| ClickHouse CPU-s per hour | Old | New |
|---|---|---|
| Inserts (Insert + AsyncInsertFlush) | 258 (185 + 73) | 182 (82 + 100) |
| Merges, all tables | 236 | 203 |
| — of the metric tables | 107 (series 59, 5m 18, 1h 17, samples 14) | 77 (tags 39, series 19, samples 15, tag values 2, 5m 1) |
| Selects (server jobs only, no UI; new includes the rollup job) | 4.0 | 4.4 |
| **Total** | **498** | **390 (−22 %)** |

- Bytes written by the metric tables: **4.3× lower** (3.1 GiB → 0.7 GiB per hour); series 10.5×, rollups 17–200×.
- Rows are not lower: 3.97 → 3.84 rows per point. The tag index writes 47.7 M rows/h (21 per series row), and the
  series table still gets 2.28 M rows/h (25× the 90k active series): OTLP points (the load's 10 %, ≈ 1.9 M/h, plus
  `oc-tracegen`'s OTLP host and application metrics) are not marked (Stage 2, "not done"), so each is a series row
  with ~20 index rows. Stage 2's targets (series rows ≤ 2 × active series, metric merge CPU ≥ 3× lower) were met
  on a Datadog-only load; with 10 % OTLP they are **not met** (series 25× active series, metric merges 1.4×
  lower). Marking OTLP in the intake remains the largest saving left, as Stage 7 found.

### Page timings

Requests as the web app sends them (6 connections, session cookie). Cold: server restarted, Valkey `FLUSHALL`
(new), ClickHouse mark / index mark / uncompressed / query caches dropped, 10 s to settle. Warm: the same load
right after. ClickHouse columns: SELECTs of the server's user started within the load, medians.

Kubernetes - Overview (53 metric queries over the live stream, 5 template variables, events, 6 inventory widgets;
total until the last answered), 5 cold + 5 warm runs each:

| | Old median / p90 | New median / p90 | Old ClickHouse per load | New ClickHouse per load |
|---|---|---|---|---|
| Cold | 2,370 / 2,524 ms | **1,125 / 1,161 ms** | 42 queries, 16.3 M rows, 886 MiB, 3.9 CPU-s | 68 queries, 10.9 M rows, 290 MiB, 1.8 CPU-s |
| Warm | 2,165 / 2,283 ms | **84 / 567 ms** | 42 queries, 15.3 M rows, 770 MiB, 3.7 CPU-s | 6 queries, 0.11 M rows, 0.9 MiB, 0.02 CPU-s |
| 3 users at once, cold (1 round) | 6,067 ms (each 6.0–6.1 s) | **1,279 ms** (each 1.23–1.28 s) | 120 queries, 40 M rows, 2.3 GiB, 10.2 CPU-s | 78 queries, 12.7 M rows, 328 MiB, 2.0 CPU-s |
| 3 users at once, warm | 6,232 ms | **144 ms** (97–144 ms) | 120 queries, 41 M rows, 2.6 GiB, 10.6 CPU-s | 16 queries, 0.1 M rows, 1 MiB, 0.06 CPU-s |

- Where the time goes (medians): old — metric stream 2.28 s, inventory widgets 1.5–2.3 s, tag values 120–220 ms;
  new cold — metric stream 1.09 s (the long pole), inventory widgets 680–700 ms (one shared inventory build), tag
  values 12–13 ms, events 19 ms; new warm — stream 66 ms, inventory 21–22 ms.
- New's warm p90 (567 ms; one run 888 ms) is a load that crossed a minute boundary: the page's range moved by a
  minute and part of the widgets missed their entries (TTL min(step, 30 s)). Old's warm load is no faster than its
  cold one: it has no result cache.
- Three users at once cost old 2.6× one user (queries queue on 2 vCPU); new shares one execution (coalescing), so
  three cold users take 1.1× one.
- New sends 68 queries cold against old's 42 (one per widget instead of shared scans) but reads 33 % fewer rows
  and 3× fewer bytes at half the CPU.

Other pages (3 cold + 3 warm runs each, medians, p90 in brackets):

| Page / request | Old cold | Old warm | New cold | New warm | Target |
|---|---|---|---|---|---|
| Template variable values (each of 9 keys) | 44–69 ms (≤ 81), 109–135 k rows | 31–46 ms | **8–12 ms** (≤ 12), 5–9 k rows | 2–4 ms | < 100 ms: met |
| Kubernetes Explorer, pods (50, by name) | 321 (322) ms, 1.7 M rows, 136 MiB | 331 (362) ms | **316 (342) ms**, 1.1 M rows, 68 MiB | 26 (27) ms, no query | < 300 ms: met warm, **missed cold** |
| Errors (issues + facets + users, 1 day) | 612 (733) ms, 15 M rows, 1.08 CPU-s | 601 (620) ms | **424 (470) ms**, 11 M rows, 0.77 CPU-s | 418 (485) ms | < 500 ms: met, narrowly |
| Log Explorer facets (1 h) | 608 (612) ms, 4.4 M rows, 0.77 CPU-s | 382 (436) ms | **130 (149) ms**, 1.3 M rows, 0.18 CPU-s | 167 (170) ms | < 300 ms: met |
| Metrics Explorer `avg:kubernetes.cpu.usage.total{*} by {host}`, 1 h | 140 (186) ms | 106 (153) ms | 90 (90) ms | 92 (99) ms | |
| same, 24 h | 50 (60) ms | 39 (51) ms | 43 (47) ms | 32 (34) ms | |
| Metrics Explorer `avg:system.cpu.user{*} by {host}`, 1 h | 62 (66) ms | 42 (82) ms | **121 (130) ms** | 38 (40) ms | |
| same, 24 h | 52 (52) ms | 38 (50) ms | 53 (56) ms | 50 (54) ms | |

(Metrics Explorer = the metric catalog `GET /api/v1/metrics` and `POST /api/v1/query/timeseries` together; the
first Metrics Explorer row of a cold run pays the catalog.)

Regressions and open points:
- **Metric catalog, cold** (`GET /api/v1/metrics`): 121 ms on new against 62 ms on old. Its query
  (`SELECT MetricName, toString(MetricType), any(MetricUnit), any(MetricDescription) FROM otel.metrics_series WHERE
  OrgId = … GROUP BY …`, joined with the attribute keys) reads 4.47 M rows of `metrics_series` (106 ms, 147 ms CPU)
  against ~0.45 M rows for old's whole page: the series table holds the unmarked OTLP rows (2.3 M/h). Warm: 4 ms
  (Valkey, 60 s).
- **`POST /api/v1/query/timeseries` for `avg:system.cpu.user{*} by {host}`** (`oc-tracegen`'s OTLP host metrics,
  6 hosts): 38–55 ms on new against 21–47 ms on old in every mode (24 h warm 50 against 27 ms). Small in absolute
  terms; `kubernetes.cpu.usage.total` (3,200 pods, Datadog + OTLP) is faster on new.
- **Errors page**: both versions spend ~340 ms in the APM half: `source: all` reads span errors from
  `otel_traces` (~2.8 M rows, ~330 ms CPU, issues and facets each), while the log half (Stage 6's aggregates) takes
  15–16 ms. With one hour of spans the page is at 424 ms; the span scan grows with the range, so with dev's day of
  spans the < 500 ms target will not hold. Span error groups need the same aggregate as the logs.
- **Pods explorer, cold**: 316 ms (p90 342), just over the 300 ms target; the inventory build (5 queries, 1.1 M
  rows, 68 MiB) is paid once per 15 s per org, after which the page answers in 26 ms.

Targets: Kubernetes Overview < 2 s at 2 vCPU **met** (1.13 s cold, 0.08 s warm; old 2.4 s here, dev 15–20 s); tag
values < 100 ms **met** (8–12 ms cold); pods explorer < 300 ms met warm (26 ms), **missed cold by 16 ms**; Errors
< 500 ms **met** (424 ms) with the span-errors caveat; log facets < 300 ms **met** (130–167 ms); write
amplification: bytes 4.3× lower and ClickHouse CPU −22 %, but **rows per point unchanged** (3.97 → 3.84) under 10 %
OTLP, because of the unmarked OTLP points and the tag index.

Limitations:
- 52 minutes of warm-up against dev's days of data: series, rollups and tag tables are an hour old, merges see small
  parts only (dev's merges rewrite a day of series and rollups), and the 24 h queries cover one hour of data. Old's
  per-point series and rollup rows make its reads grow with history, so the gap should widen on dev; the span
  scan of the Errors page grows on both.
- One laptop for both stacks, the senders and the measuring client, with other local workloads running. ClickHouse is
  capped at 2 vCPU each; servers and intakes are not.
- `oc-tracegen`'s spans and logs are random per process (statistically equal across the stacks).
- One concurrent round, three runs per page (five for the dashboard): a p90 of three runs is close to the maximum.
- No browser: the requests are replayed with the web app's own query planning (`collectDashboardQueries`,
  `resolveQuery`); rendering is not included.
