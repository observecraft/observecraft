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
  types are partial: marking it needs lossless splitting). Cluster run of the new DDL not done here (the
  rendered DDL is unit-tested in both modes).

Status: Complete (single node)

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
  `distributed_product_mode = 'local'`. Rendered DDL unit-tested; not run on a real cluster here.

Status: Complete (single node)

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

Status: Not Started

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

Follow-up: switch call sites after Stages 2–4: dashboard widget results, template variable / tag values and the
metric catalog through `cache.get_or_load`, dashboard edits through `cache.invalidate`; the Kubernetes inventory's
`ttl_cache` load through the shared cache (one replica builds, the others decode its copy); `/metrics` reachable
by a scraper (NetworkPolicy) when we monitor ObserveCraft with itself.

Status: Complete (infrastructure and module; call sites pending)
