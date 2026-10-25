-- Log indexes and per-row retention.
--   oc.logs      indexed logs: each row carries its index and how many days it is kept
--   oc.logs_raw  every ingested log for a short while (archives read it, so they also get logs that no
--                index keeps), regardless of indexes, quotas and exclusion filters
ALTER TABLE oc.logs ADD COLUMN IF NOT EXISTS index_name LowCardinality(String) DEFAULT 'main';
ALTER TABLE oc.logs ADD COLUMN IF NOT EXISTS retention_days UInt16 DEFAULT 15;
ALTER TABLE oc.logs MODIFY TTL toDateTime(timestamp) + toIntervalDay(retention_days);
CREATE TABLE IF NOT EXISTS oc.logs_raw AS oc.logs;
ALTER TABLE oc.logs_raw MODIFY TTL toDateTime(timestamp) + INTERVAL 3 DAY;

-- Spans: the org's retention is stamped on the resource by the intake/collector (oc.retention_days);
-- 0 (not stamped) means the install default (OC_RETENTION_TRACES_DAYS).
ALTER TABLE otel.otel_traces ADD COLUMN IF NOT EXISTS RetentionDays UInt16 MATERIALIZED toUInt16OrZero(ResourceAttributes['oc.retention_days']);

-- Rehydrated logs keep the same columns (their index is the rehydration).
ALTER TABLE oc.logs_rehydrated ADD COLUMN IF NOT EXISTS index_name LowCardinality(String) DEFAULT 'rehydrated';
ALTER TABLE oc.logs_rehydrated ADD COLUMN IF NOT EXISTS retention_days UInt16 DEFAULT 0;
