# ADR-0001: Metrics storage for per-organization retention

**Status:** Proposed
**Date:** 2026-10-27
**Deciders:** maintainers

## Context

Metrics are stored as the OpenTelemetry Collector's ClickHouse exporter writes them: one row per data point in
`otel.otel_metrics_{gauge,sum,histogram,exponential_histogram,summary}`, with the full `ResourceAttributes` and
`Attributes` maps on every row. These tables are partitioned by day, sorted by
`(OrgId, MetricName, ServiceName, Attributes, TimeUnix)`, and expire after one install-wide TTL (30 days by default).

We want per-organization retention, including long retention (a year or more) that an organization can pay for. It
has to work for a multi-tenant SaaS and for single-organization installs, and long ranges must not make queries
slower.

Measured on one real Kubernetes cluster (≈4,200 points/s; one hour sample: 19M points, 190k series, 681 metrics):

| Layout | Bytes per point | One dashboard query (1h, avg by namespace) |
|---|---|---|
| Current tables, before collector 0.163 (map keys in random order) | 52 | — |
| Current tables, map keys in a stable order | 3.3 | 263 ms, 229 MB read |
| Series stored once + samples (series id, time, value) | 1.6 (+ series, which doesn't grow with time) | 18 ms, 12 MB read |
| 5-minute rollup of the samples | 0.5 | 5 ms, 3 MB read |
| 1-hour rollup of the samples | 0.2 | — |

Bytes per point divide each layout's size by the raw points it covers.

Forces:

- **Query cost grows with the range.** Every query reads raw points today. The API caps ranges at 31 days. A year
  of one metric is ~2 TB read in the current layout and ~100 GB as raw samples, so a year is only practical from
  rollups.
- **Most of each row is labels.** Reading `ResourceAttributes[...]` decompresses the whole map on every row, which
  is 90% of the bytes read.
- **Retention must drop whole parts.** Dropping rows inside parts means rewriting them continuously. That only works
  when a part holds a single retention, i.e. retention is part of the partition key. A partition key can't be changed
  with `ALTER`, so per-org retention needs new tables either way.
- **TTL changes don't reach existing parts.** Retention is applied with `materialize_ttl_after_modify = 0`, and
  existing parts keep the TTL they were written with, in both directions. Lowering retention frees nothing, and
  raising it still deletes old data on the old schedule. We verified this on ClickHouse 24.8.
- **Sharding and series ids disagree.** In a cluster, metrics are sharded by `cityHash64(..., toString(Attributes),
  toString(ResourceAttributes))` over maps in arrival order, but queries identify a series by its sorted maps.
- **Constraints** (CLAUDE.md):
  - stay OpenTelemetry-native, with every sender landing in the same ClickHouse tables (no parallel pipeline);
  - keep Datadog metric names;
  - single-node and clustered DDL;
  - every query filtered by org.

## Decision

Store metrics as **series + samples + rollups**, populated inside ClickHouse from the exporter's inserts, with the
retention of each organization in the partition key. ClickHouse stays the only store.

This is ClickHouse's own layout for metrics:

- **Same shape as the `TimeSeries` engine.** Its [`TimeSeries` table engine](https://clickhouse.com/docs/engines/table-engines/special/time_series)
  has a tags table (`ReplacingMergeTree`, one row per series), a samples table ordered by `(id, timestamp)` with
  Delta/T64 on time and ALP on values, and an id hashed from the metric name and tags. It is a private preview
  ("may change in backwards-incompatible ways"), so we build the same layout on plain MergeTree tables we control.
- **Ingest pattern.** A `Null` table receiving inserts, with materialized views writing the optimized tables, is
  the pattern the [observability schema guide](https://clickhouse.com/docs/use-cases/observability/schema-design)
  recommends. The guide also explains why we take labels out of the per-point rows: "access to the map keys and
  their values isn't as efficient as access to a normal column."
- **Rollups.** Materialized views into aggregating tables, as in the
  [time-series query guide](https://clickhouse.com/docs/use-cases/time-series/query-performance).
- **Server version.** **ClickHouse 26.8 LTS** (stage 0). 24.8 no longer gets fixes; only 26.3 and 26.8 are
  supported LTS releases, and 26.8 has the ALP codec. Our migrations, the bundled tuning, the ingest profile and
  collector 0.163.2 all work on 26.8. We verified that end to end.

1. **Ingest.** The collector's exporter writes to `otel.metrics_ingest_*` tables with the `Null` engine, set
   through the exporter's `metrics_tables` names. Nothing is stored there; materialized views fan each insert out:
   - `SeriesId = cityHash64(OrgId, MetricName, mapSort(ResourceAttributes), mapSort(Attributes))`. It is computed
     in ClickHouse, so it no longer depends on the order a sender or exporter uses.
   - `RetentionDays` comes from a resource attribute the intake stamps (`oc.metrics_retention_days`), separate from
     span retention.
2. **`otel.metrics_series`**: one row per series, holding the sorted maps, ServiceName, unit, type and temporality,
   with `LastSeen`.
   - Engine: `ReplacingMergeTree(LastSeen)` ordered by `(OrgId, MetricName, SeriesId)`.
   - TTL: `LastSeen` + the org's longest tier.
   - Readers filter and group on this small table, then join samples by `SeriesId`.
3. **`otel.metrics_samples`** (gauge and sum): `(OrgId, MetricName, SeriesId, TimeUnix, Value, RetentionDays)`.
   - Codecs: Delta-family on time; `Gorilla` or `ALP` on values, whichever compresses real samples better
     in stage 1 (`ALP` needs 26.x).
   - `ORDER BY (OrgId, MetricName, SeriesId, TimeUnix)`, `PARTITION BY (toDate(TimeUnix), RetentionDays)`.
   - `ttl_only_drop_parts = 1`.
   - Distributions get their own samples tables (buckets as arrays) with the same keys.
4. **Rollups `metrics_rollup_5m` and `metrics_rollup_1h`**: `AggregatingMergeTree` with min, max, sum, count,
   first and last per series and bucket, fed by materialized views.
   - Retention has its own tiers.
   - Partitioning: by day (5m) and by month (1h), times retention, which keeps the partition count bounded.
5. **Per-org retention.** Each organization has three durations: raw, 5m and 1h.
   - Each is chosen from a fixed set of tiers (e.g. 7, 15, 30, 90, 180, 395, 760 days), within limits the instance
     admin sets. That is the SaaS plan; a single-org install just sets its own.
   - A fixed set of tiers keeps partitions bounded.
   - A change applies to data written from then on, as log index and span retention already do.
6. **Resolution picked by the query.** The query layer reads raw samples while the range is within the org's raw
   tier and the step is under 5 minutes, then 5m, then 1h. The 31-day cap becomes the org's retention.
7. **Cluster.** Every new table is sharded by `cityHash64(OrgId, MetricName, SeriesId)`, so a series and its
   rollups live on one shard.

## Options considered

### A: Keep the exporter's tables, rebuild them partitioned by retention

| Dimension | Assessment |
|---|---|
| Complexity | Low–Medium (one rebuild, same readers) |
| Cost | 3.3 B/point; labels decompressed on every row read |
| Scalability | Ranges beyond weeks stay slow (no rollups); a year is impractical |
| Team familiarity | High |

**Pros:** smallest change; readers untouched.
**Cons:** doesn't solve long-range queries; every query keeps reading the maps; we're still tied to the exporter's
schema and its column order.

### B: Series + samples + rollups in ClickHouse (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium–High (new tables and MVs, query layer rewrite, migration window) |
| Cost | ~1.6 B/point raw, 0.2–0.5 B/point in rollups |
| Scalability | Year-long queries read only rollups; retention drops whole partitions; shards keep series together |
| Team familiarity | Medium (MVs and AggregatingMergeTree are standard ClickHouse) |

**Pros:** Prometheus-class storage; 15–50× less read per query; per-org retention without rewrites; series catalog
and tag values become cheap reads of the series table.
**Cons:** every metrics reader changes; joins series → samples; rollups add write amplification (~10%); old data
needs a transition plan.

### C: A separate TSDB (e.g. VictoriaMetrics, Apache-2.0)

| Dimension | Assessment |
|---|---|
| Complexity | High (second storage system to run, back up, secure and shard per tenant) |
| Cost | Very low per point |
| Scalability | Proven |
| Team familiarity | Low |

**Pros:** purpose-built compression and downsampling.
**Cons:** a parallel pipeline next to ClickHouse, against the project's OTel-native, single-store direction; metrics
can no longer be joined with spans and logs in SQL; two query languages to keep compatible. (Mimir/Thanos-style
stores under AGPL are excluded by license.)

### D: Fixed retention plus rollups, no per-org choice (Datadog's model: 15 months for everyone)

| Dimension | Assessment |
|---|---|
| Complexity | Medium |
| Cost | Same as B |
| Scalability | Same as B |
| Team familiarity | Medium |

**Pros:** simpler product; what Datadog does.
**Cons:** no paid long-retention tiers; single-org installs can't trade disk for history.

## Trade-off analysis

Option A is the cheapest change, but it doesn't meet the requirement. Partitioning fixes deletion, but long ranges
still read raw points and maps. Options B and D share the storage work, and per-org tiers only add a partition-key
column and settings, so B costs little more than D and keeps the paid-retention option. Option C would perform well,
but it breaks the single ClickHouse store that the logs, traces and metrics correlation depends on.

The main costs of B are rewriting the query layer and the transition. Both can be staged behind the existing
`metrics.rs` builders, which already compute a sorted-map series id and per-series buckets.

## Consequences

- **Easier:**
  - long ranges (dashboards over months, SLO windows);
  - per-org paid retention;
  - cheaper metric catalog and tag-value lookups from the series table;
  - sender-independent series identity;
  - correct sharding.
- **Harder:**
  - every new metrics reader has to go through series + samples or rollups;
  - schema changes to the exporter can't break us anymore, but we now own the ingest DDL and its MVs.
- **Revisit:**
  - **Counter resets in rollups.** first/last per bucket handle the common case; check them against real counters.
  - **Series churn.** Short-lived pods mean many series. Measure how big the series table gets per day in SaaS.
  - **Old data.** Either dual-read the old tables until their TTL ends, or backfill.
  - **Rollup bucket sizes.** Confirm 5m and 1h with real dashboards.
  - **Retention changes on existing data.** Today they don't apply. If they should, they need a partition copy.

## Action items

See `IMPLEMENTATION_PLAN.md`.

0. [ ] ClickHouse 26.8 LTS (24.8 is out of support).
1. [ ] Ingest tables, series, samples, rollups and MVs (single node and cluster), with tests.
2. [ ] Query layer on series + samples + rollups, with resolution selection and parity tests against today's results.
3. [ ] Remaining readers (hosts, Kubernetes, catalog, tag values, monitors).
4. [ ] Per-org metrics retention: settings, intake stamping, UI, instance limits.
5. [ ] Distributions on the same model; transition off the old tables.
6. [ ] Fix the TTL-on-existing-parts behaviour for the tables that keep install-wide TTLs.
