-- Each profile's total per sample type (cpu-time, wall-time, alloc-space, …), computed at ingestion,
-- so CPU-over-time charts, service totals and per-minute averages don't decode every pprof file.
ALTER TABLE otel.profiles${LOCAL} ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS `SampleTotals` Map(LowCardinality(String), Int64) AFTER `Tags`;
-- In a cluster the Distributed table needs the column too (on a single node this is the same table).
ALTER TABLE otel.profiles ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS `SampleTotals` Map(LowCardinality(String), Int64) AFTER `Tags`;
