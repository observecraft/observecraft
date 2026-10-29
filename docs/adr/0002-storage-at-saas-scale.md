# ADR-0002: Telemetry storage that scales with organizations and data

**Status:** Proposed
**Date:** 2026-10-29
**Deciders:** maintainers

## Context

ADR-0001 moved metrics to series + samples + rollups. The layout is right: on one real cluster a dashboard query
read 12 MB in 18 ms. But on our dev environment (one AKS cluster, 14 nodes, the Datadog Agent, ClickHouse 26.8 with
2 vCPU and 6 GiB), Kubernetes dashboards take 15–20 s, and ClickHouse is busy most of the time. Measured on dev,
one hour, 2026-10-29:

**Volume.** ~5,000 points/s (308k/min), ~90k active series, ~524k series over the retention (pods and containers
come and go), 750 metric names, ~21 labels per series (~880 bytes as text).

**Writes.** Rows written in one hour:

| Table | Rows | Bytes written |
|---|---|---|
| `metrics_samples` | 18.5 M | 214 MiB |
| `metrics_series` | 18.2 M | 911 MiB |
| `metrics_rollup_5m` | 18.5 M | 448 MiB |
| `metrics_rollup_1h` | 18.5 M | 448 MiB |

For every point we write a series row (with both label maps) and two rollup rows: ~2 GiB to keep 214 MiB of
points, about 9× write amplification. The cause: materialized views run once per insert block, and a block (~110
inserts/min) holds one point per series, so their `GROUP BY` collapses nothing. Merging those rows back is the
largest CPU consumer.

**CPU, one hour.** Inserts 496 s, merges 601 s, selects 557 s. The heaviest selects:

| Query | CPU | Why |
|---|---|---|
| Errors page facets (`uniqExact` over `ARRAY JOIN` of logs) | 166 s / 12 runs | recomputed from raw logs on every load |
| Error groups (`argMax` over logs) | 144 s / 17 runs | same |
| Log facet counts | 62 s / 173 runs | scans every matching log row per facet |
| Dashboard batch (30 widgets in one query) | 4.9 s, 3.2 M rows, 100 MB per run | tag filters evaluated over label maps of every series; `uniqExact` and `medianExactIf` per series and interval |
| Template variable values (`DISTINCT ResourceAttributes[k]`) | 3–3.5 s each | scans the whole series table (~760k rows with duplicates) for every key, on every load |
| Kubernetes inventory (`k8s_resources FINAL`, JSON bodies) | 3–5 s, 110 MB, ×6 per dashboard | every pod/workload widget reads and parses all objects, then filters in Rust |

None of these costs depends on what a page shows; they grow with the total data of the organization, and the
write costs grow with total ingest. Both are what breaks a multi-tenant SaaS first.

Forces and constraints:

- Horizontal scale: more shards must mean proportionally more ingest and query capacity, with every org's reads
  spread over the shards and no shard holding a whole large org.
- Tenancy: every table leads with the org; every read filters on it (CLAUDE.md).
- Single-node and cluster DDL (the `${...}` placeholders), OpenTelemetry-native ingest, Datadog metric names kept.
- Dev data can be dropped: we may recreate the `otel` database instead of migrating it.

### How Datadog approaches the same problems

From Datadog's public engineering material, at the level it is described there:

- **Tags are indexed separately from points.** An index answers "which series match these tags" and lists tag
  values; the point store reads only those series.
- **Aggregation happens before storage.** The Agent aggregates in flush intervals (DogStatsD) and sends
  distributions as sketches; long ranges are served from rollups computed in the background, not at query time.
- **Cardinality is configured.** Metrics without Limits indexes only the tag combinations an org configures.
- **Recent data is hot, history is cheap.** Real-time storage holds recent points; older data lives in cheaper
  storage. Query results are cached.
- **Explorers query indexed fields**, not documents parsed per request.

The common principle: do work once at ingest or in the background, so a query reads only what it shows.

## Decision

Keep ClickHouse as the only store and the ADR-0001 layout, and change *when* work is done:

### 1. Write path: one series row per series and hour, rollups computed in the background

- **Series rows only when needed.** Every metric enters through the intake (ingest-key auth). Org settings
  travel as request headers that the Collector turns into `oc.*` resource attributes for the whole request. The
  intake keeps a bounded per-replica cache of the series it forwarded in the current hour (hash of org, metric,
  type and sorted labels) and sends points of known series in a separate request with `x-oc-series-known: 1`,
  which becomes `oc.series_known = 1`. `oc.*` attributes are not part of a series' identity, so samples are
  unchanged; the series view skips marked points. Result: at most one series row per series, hour and intake
  replica (dev: ~90k–180k rows/h instead of 18 M). A cache eviction or restart only writes an extra row, which the
  `ReplacingMergeTree` absorbs.
  - Datadog-format metrics are converted to OTLP by the intake, so splitting them costs nothing extra; this is
    all of dev's metrics.
  - OTLP metrics are forwarded undecoded today (gRPC passed through, HTTP bodies as sent). Marking them means
    decoding and re-encoding in the intake; Stage 1 measures that cost. Until then they keep one series row per
    point, as today.
- **Rollups by a background job, not per-insert views.** A rollup job in `observecraft-server` (claimed through
  Postgres like the other schedulers, so one server runs it) inserts each closed 5-minute bucket from
  `metrics_samples` once it is past a lateness window, and each hour from the 5-minute rollup. Rollups become
  `ReplacingMergeTree` tables with plain min/max/sum/count/first/last values, so re-running a bucket (late data,
  a retry) replaces it instead of double-counting. Rows written: one per series and bucket (dev: ~1.1 M/h for 5m,
  ~90k/h for 1h, instead of 18.5 M each). Queries read rollups up to the last completed bucket and raw samples for
  the tail.
- **Distributions** (histograms, exponential histograms, summaries) follow the same two rules.
- Expected write amplification: from ~9× to ~1.2×.

### 2. Read path: indexes and precomputed answers

- **Tag index** `otel.metrics_tags (OrgId, MetricName, Key, Value, SeriesId, LastSeen)`,
  `ReplacingMergeTree`, `ORDER BY (OrgId, MetricName, Key, Value, SeriesId)`, fed by a view from
  `metrics_series` (cheap now that series rows are rare). A filter like `kube_namespace:billing` becomes a
  primary-key range; the query then reads samples with `MetricName = ? AND SeriesId IN (…)`.
- **Tag values** `otel.metrics_tag_values (OrgId, Key, Value, LastSeen)`, `ReplacingMergeTree`
  `ORDER BY (OrgId, Key, Value)`: template variables and facet lists become one small range read.
- **Dashboard queries per widget, in parallel, cached.** Replace the 30-in-1 query with one query per widget (each
  now cheap), a short result cache keyed by org, query, range and step, and request coalescing so concurrent
  identical queries share one execution. Drop the per-series, per-interval `uniqExact`/`medianExactIf`; derive the
  interval from the series' sample count and span, as the rollups do. Parity tests against today's results.
- **Kubernetes inventory as columns.** `k8s_resources` gains the fields explorers filter and sort on (namespace,
  phase, node, owner, ready, restarts, labels as a map, CPU/memory requests); explorers and widgets filter, sort and
  paginate in SQL and read `Body` only for the detail panel. A short per-org cache with coalescing serves the
  dashboard's several inventory widgets with one read.
- **Errors precomputed.** Error groups (service, kind, key, counts per minute, first/last seen, a sample message)
  are maintained at ingest by a view from the indexed logs into an aggregating table; the Errors page and its facets
  read that table.
- **Log facets.** Counts for facets over large ranges come from a per-minute aggregate of the standard facets
  (service, status, host, source, index) fed by a view; free-form attribute facets keep scanning, bounded by the
  page's time range and a row limit with an "approximate" marker.

### 3. Horizontal scale

- **Sharding stays by series** (`SeriesId`, spans by `TraceId`, Kubernetes objects by uid, logs evenly), so a large
  org spreads over all shards; every shard keeps each org's data together in sort order.
- **Co-located joins.** Series, tags, samples and rollups shard on the same `SeriesId`, so series → samples joins
  run shard-locally (`distributed_product_mode = 'local'`); only the small results travel.
- **Replicas** for read capacity and availability (`ReplicatedMergeTree`, Keeper), unchanged.
- **Per-org protection.** Per-org query limits (time, memory, concurrent queries) through ClickHouse settings
  profiles applied per request, on top of the intake's per-org rate limits.
- **Sizing.** At least 4 vCPU per ClickHouse node; the dev values move from 2 to 4.

### 4. Rebuild instead of migrating

New numbered migrations create the new tables and views and drop the old views and the unused exporter tables
(`otel.otel_metrics_*`). Data is not migrated: dev's `otel` database is recreated, and any install upgrading keeps
logs and spans and starts metrics fresh (as ADR-0001 already did).

## Options considered

### A: More hardware, no changes

| Dimension | Assessment |
|---|---|
| Complexity | None |
| Cost | Grows linearly with ingest (9× write amplification stays) |
| Scalability | Buys time; per-request scans keep growing with each org's data |
| Team familiarity | High |

### B: Ingest-time and background work, indexes (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium–High (intake cache, rollup job, new tables, query layer changes) |
| Cost | ~1.2× write amplification; reads proportional to what a page shows |
| Scalability | Adds shards linearly; reads stay bounded per page |
| Team familiarity | Medium (MVs, ReplacingMergeTree, schedulers already exist) |

### C: Refreshable materialized views for rollups instead of a job

| Dimension | Assessment |
|---|---|
| Complexity | Low in DDL |
| Cost | Same as B |
| Scalability | In a cluster every replica refreshes unless the database is `Replicated`; late data handling is coarse |
| Team familiarity | Low |

Rejected for now: we'd depend on `Replicated` databases for correctness. Revisit when we adopt them.

### D: A separate TSDB / index service (VictoriaMetrics, a Rust index)

Rejected as in ADR-0001: a second store and pipeline next to ClickHouse.

## Trade-off analysis

A is what we pay today and grows with every customer. B moves the same work to where it happens once: the intake
knows which series it has seen, a background job knows which buckets are closed, and indexes answer lookups without
scans. Its main risks are correctness of the rollup job (late points, reruns) and parity of the query layer; both
are testable against the current results before switching. C would save the job's code but makes cluster
correctness depend on database engines we don't use yet.

## Consequences

- **Easier:** ingest grows ~linearly with shards; dashboards and explorers read in proportion to what they show;
  template variables, facets and the Errors page become constant-time reads; noisy orgs are bounded.
- **Harder:** the intake carries a cache; the rollup job is a new moving part to monitor (lag, reruns); one more
  index table to keep consistent.
- **Revisit:** Metrics without Limits-style per-org tag allowlists on the index; ALP codec when GA; hot/cold
  storage for samples older than the raw tier; shard groups per very large org.

## Action items

See `IMPLEMENTATION_PLAN.md`.
