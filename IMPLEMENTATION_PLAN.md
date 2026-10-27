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
- Migration `0006` adds the following, with the `${...}` placeholders for cluster mode:
  - `otel.metrics_ingest_*` (Null engine);
  - `otel.metrics_series`;
  - `otel.metrics_samples` (+ distribution samples);
  - `otel.metrics_rollup_5m` and `otel.metrics_rollup_1h`;
  - the materialized views between them.
- The collector writes to both the old and the new tables during the transition: a second exporter with
  `metrics_tables` names.

Success criteria:
- A test inserting OTLP-shaped rows into the ingest tables finds:
  - one series per distinct label set, whatever the key order;
  - every point in samples;
  - rollup min, max, sum, count, first and last equal to the same aggregation over samples.
- The migration is idempotent, and the cluster test shards a series and its rollups onto one shard.
- The e2e run (collector 0.163 + telemetrygen) fills all new tables with no exporter errors.

Status: Not Started

## Stage 2: Query layer on the new tables

Goal:
- `routes/metrics.rs` (`inner_query`, `dist_query`, `metric_kind`) reads series + samples or rollups.
- A resolution picker chooses raw, 5m or 1h from the range, the step and the org's tiers.
- Ranges up to the org's retention replace the 31-day cap for metrics.

Success criteria:
- Parity tests show that the same data gives the same series and values through the old and new paths, within
  rounding for rollups.
- Unit tests cover resolution selection.
- A dashboard over 1h reads ≥10× fewer bytes than today (measured as in the ADR).

Status: Not Started

## Stage 3: Remaining readers

Goal:
- `hosts.rs`, `kubernetes.rs`, the metric catalog and `tag_values` read the series table.
- Monitors go through the stage 2 path.

Success criteria:
- Existing route tests pass.
- Host, Kubernetes and monitor pages show the same data in live and mock mode.
- The catalog and tag values no longer scan a day of points.

Status: Not Started

## Stage 4: Per-organization metrics retention

Goal:
- Postgres: per-org raw, 5m and 1h retention, plus per-org limits set by the instance admin.
- Intake stamps `oc.metrics_retention_days` (raw, 5m, 1h).
- UI: Organization Settings → Data Retention gets metrics; Instance → Orgs → Retention limits gets the metric limits.

Success criteria:
- API tests reject values outside the tiers and the limits.
- New rows carry the org's tiers.
- Partitions per tier expire by dropping whole parts.
- The UI shows the change applies to new data.

Status: Not Started

## Stage 5: Distributions, transition and TTL fix

Goal:
- Histograms, exponential histograms and summaries on the same model.
- The old `otel.otel_metrics_*` tables are read only for ranges before the cutover, until their TTL empties them,
  then dropped.
- Install-wide TTL changes reach existing parts (`MATERIALIZE TTL` with `materialize_ttl_recalculate_only`).

Success criteria:
- Percentile and heatmap tests match the old path.
- A query across the cutover returns continuous series.
- Raising or lowering an install-wide TTL affects existing parts (test on ClickHouse 24.8).

Status: Not Started
