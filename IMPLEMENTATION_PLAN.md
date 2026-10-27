# Implementation plan: metrics storage for per-organization retention

Design: [docs/adr/0001-metrics-storage.md](docs/adr/0001-metrics-storage.md). Each stage is its own PR, and
`make test` passes at the end of every stage. Single-node and cluster DDL are both covered by tests
(`oc-store/tests/cluster.rs`).

## Stage 0: ClickHouse 26.8 LTS

Goal: the bundled and operator ClickHouse (and Keeper) move from 24.8, which no longer gets fixes, to 26.8 LTS in
both charts and compose.

Success criteria:
- Migrations apply on a fresh 26.8 and on an upgraded 24.8 → 26.8 volume.
- The bundled tuning and the ingest profile load.
- The collector e2e run fills every table with no errors.
- The cluster test passes on 26.8.

Verified:
- A fresh 26.8 applies the migrations, loads the bundled tuning and the ingest profile, and the collector
  0.163.2 e2e run fills every table with no errors.
- Upgrading a 24.8 volume to 26.8: all 1445 rows readable, migrations re-run cleanly, new writes land.
- `oc-store/tests/cluster.rs` passes on 2 shards + Keeper 26.8.
- Not covered: the Altinity operator itself; operator mode needs a Kubernetes run.

Status: Complete

## Stage 1: Storage tables and ingest views

Goal:
- Migration `0006_metrics_storage` adds, with the `${...}` placeholders for cluster mode:
  - `otel.metrics_ingest_*`: Null tables for all five point types, sharded by series in a cluster;
  - `otel.metrics_points`: a Null table where gauge and sum are normalized once;
  - `otel.metrics_series`, `otel.metrics_samples`, `otel.metrics_rollup_5m`, `otel.metrics_rollup_1h`;
  - the materialized views between them.
- The collector writes to both the old and the new tables: a second exporter, `clickhouse/metrics`, with
  `metrics_tables` names.
- Install-wide defaults for the rollups: `retention.metrics5mDays` (90) and `retention.metrics1hDays` (395).

Success criteria:
- A test inserting OTLP-shaped rows into the ingest tables finds:
  - one series per distinct label set, whatever the key order;
  - every point in samples;
  - rollup min, max, sum, count, first and last equal to the same aggregation over samples.
- The migration is idempotent, and the cluster test shards a series and its rollups onto one shard.
- The e2e run (collector 0.163 + telemetrygen) fills all new tables with no exporter errors.

Verified:
- `oc-store/tests/metrics_storage.rs` on 26.8:
  - the same labels in another key order are one series;
  - a gauge and a sum sharing a name are two series;
  - every point lands in samples;
  - rollups match the samples;
  - `oc.*` attributes aren't labels;
  - migrations run twice cleanly.
- `oc-store/tests/cluster.rs` on 2 shards + Keeper: samples, series and both rollups of each series are on one shard,
  and the series spread over both shards.
- e2e on 26.8 with the chart's users and ingest profile: the old and new tables get every point, with no exporter
  errors.
- Distribution types have ingest tables but no views yet (stage 5).
- The integration tests need a ClickHouse (`OC_TEST_CLICKHOUSE_URL`) and are skipped in CI, like the cluster test.

Status: Complete

## Stage 2: Query layer on the new tables

Goal:
- `routes/metrics.rs` reads gauges and sums from series + samples or the rollups:
  - `points_query` builds the SQL for either source;
  - `resolution` picks raw, 5m or 1h from the range, the step and the tiers;
  - `metric_kind` probes the series table first.
- Ranges that start before an org's data reached the new storage read the exporter's tables (the transition).
- Metric ranges may span the longest retention instead of 31 days.

Success criteria:
- Parity tests show that the same data gives the same series and values through the old and new paths.
- Unit tests cover resolution selection.
- A dashboard over 1h reads ≥10× fewer bytes than today (measured as in the ADR).

Verified:
- `new_storage_matches_the_exporter_tables`: gauge, cumulative counter with a reset and delta sum; avg, max and sum,
  with and without filters and `by`; steps 30s (samples), 10m (5m rollup) and 1h (1h rollup).
  - Identical series and values.
  - A mutation of the rollup formula makes it fail.
- Rollups carry `Start` and `End`, the first and last sample time, so counter rates over rollups equal those over
  raw points.
- On one real hour from dev (19.3M points):
  - the generated SQL of `avg:<metric>{*} by {kube_namespace}` takes 28 ms and reads 9.4 MB, against 383 ms and
    209 MB on the exporter's table;
  - all 1799 points are equal;
  - the same hour is stored in 49 MiB (samples, series and both rollups), against 1.04 GiB.
- Not done here: distributions still read the exporter's tables (stage 5).

Status: Complete

## Stage 3: Remaining readers

Goal:
- `hosts.rs` and `kubernetes.rs` select their points from `points_source`: series + samples, with the exporter's
  table's columns, once the new storage covers the window.
- The metric catalog and `tag_values` read the series table once it covers the last day.
- Monitors already go through the stage 2 path.

Success criteria:
- Existing route tests pass.
- Host, Kubernetes and monitor pages show the same data in live and mock mode.
- The catalog and tag values no longer scan a day of points.

Verified:
- `direct_readers_match_the_exporter_tables`: host-list-, pod- and process-shaped queries, the catalog and the tag
  values give identical rows from both storages.
- The real server, run twice on the same data:
  - first with the new storage empty of history (exporter's tables), then with it covering the window, as shown by
    `query_log`;
  - identical JSON for host search, host detail, host processes, Kubernetes processes, the metric catalog, tag
    values and timeseries;
  - no server errors.
- Not checked: the web pages in a browser. The API responses they render are identical.

Status: Complete

## Stage 4: Per-organization metrics retention

Goal:
- Postgres (`0017_metric_retention`): the org's raw / 5m / 1h retention and the instance admin's limits. NULL means
  the install's default (30 / 90 / 395).
- The API accepts only tiers (7, 15, 30, 90, 180, 395, 760), within the limits, with full resolution ≤ 5m ≤ 1h.
- The intake stamps `x-oc-metrics-retention-*-days` on every forwarded body; the collector turns those into the
  `oc.metrics_retention_*` resource attributes, replacing whatever a sender set.
- Queries pick the resolution and range limit from the org's tiers.
- UI: Organization Settings → Data Retention edits metrics; Instance → Orgs → Retention limits sets the metric
  limits.

Success criteria:
- API tests reject values outside the tiers and the limits.
- New rows carry the org's tiers.
- Partitions per tier expire by dropping whole parts.
- The UI shows the change applies to new data.

Verified:
- Unit tests:
  - validation (tiers, limits, order);
  - effective retention (unset = install default);
  - intake headers.
- Locally (server + intake + collector 0.163.2 + ClickHouse 26.8):
  - the API rejects non-tiers, a finer resolution kept longer, and values over the limit;
  - raising the limit from Instance works;
  - OTLP and Datadog series sent through the intake land with the org's 15 / 180 / 760 days on samples, rollups and
    series, even when the sender set `oc.metrics_retention_days` itself;
  - partitions are per tier;
  - an expired 7-day row is dropped while a 15-day row of the same age stays.
- In the browser (live mode):
  - Data Retention shows and saves the metric retention;
  - the Retention limits dialog shows and edits the metric limits.
- These settings pages have no in-browser mocks; they had none before either.

Status: Complete

## Stage 5: No transition, distributions, TTL fix

Decision (2026-10-27): existing metric data doesn't need to survive the upgrade.
- The collector writes metrics only to the new storage.
- Readers no longer fall back to the exporter's tables.
- The old `otel.otel_metrics_*` tables are left unused (not dropped by a migration) and empty out through their TTL.
- Parity tests keep the exporter-table SQL as their oracle.

Goal:
- Histograms, exponential histograms and summaries on the same model: `metrics_dist_points` → series,
  `metrics_dist_samples`, `metrics_dist_rollup_5m` / `_1h`. Rollups keep each bucket's first, last and total count,
  sum and buckets, plus min / max, the bucket layout and quantile totals.
- Install-wide TTL changes reach existing parts: `MATERIALIZE TTL` with `materialize_ttl_recalculate_only`, only
  when a table's TTL changed.

Success criteria:
- Percentile, count/sum/avg/min/max results match the oracle on the exporter's tables, at raw and rollup
  resolution, for cumulative and delta series.
- Raising or lowering an install-wide TTL affects existing parts (test on ClickHouse 26.8).

Verified:
- `distributions_match_the_exporter_tables` gives identical values, with mutations of the rollup delta and
  summary formulas failing it:
  - cumulative and delta histograms, a cumulative exponential histogram and a summary;
  - count, sum, avg, min, max, p50, p99, with and without `by`;
  - raw, 5m and 1h.
- `retention_changes_reach_existing_parts`: lowering a default drops existing expired data; raising it extends
  existing parts' expiry.
- Cluster test: distribution samples and rollups share their series' shard; retention applies on both shards.
- e2e with collector 0.163.2: histograms and exponential histograms reach the new tables, the old tables get
  nothing, no exporter errors.

Status: Complete

## Stage 6: Cold storage for old metric partitions

Goal:
- Optional tiered storage for the bundled and operator ClickHouse: an S3 / RustFS disk in a storage policy.
- The rollup tables (and optionally samples) move partitions older than N days there
  (`TTL … TO VOLUME 'cold'`) and stay queryable with the same SQL.
- Helm values for the bucket, credentials and the move age.
- Logs already have their cold path: hourly Parquet archives in the org's bucket, with Archive Search and
  rehydration.

Success criteria:
- On a local RustFS, partitions past the move age land on the S3 disk (`system.parts.disk_name`).
- The parity and range queries return the same results before and after the move.
- Without the values set, nothing changes (single disk).

Status: Not Started

