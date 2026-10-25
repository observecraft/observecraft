-- OTLP log fields: trace correlation, numeric severity and instrumentation scope.
ALTER TABLE oc.logs ADD COLUMN IF NOT EXISTS trace_id String DEFAULT '' CODEC(ZSTD(1));
ALTER TABLE oc.logs ADD COLUMN IF NOT EXISTS span_id String DEFAULT '' CODEC(ZSTD(1));
ALTER TABLE oc.logs ADD COLUMN IF NOT EXISTS severity_number UInt8 DEFAULT 0;
ALTER TABLE oc.logs ADD COLUMN IF NOT EXISTS scope LowCardinality(String) DEFAULT '';
-- "Logs of this trace" lookups.
ALTER TABLE oc.logs ADD INDEX IF NOT EXISTS idx_trace_id trace_id TYPE bloom_filter(0.001) GRANULARITY 1;
