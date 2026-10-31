-- The rollup worker scans arrival minutes exactly once, then rewinds both tiers when a closed
-- bucket received samples. Overlap ClickHouse view creation during startup by ten minutes.
-- Progress is committed in the same transaction as both rewinds.
ALTER TABLE metric_rollups ADD COLUMN arrivals_until TIMESTAMPTZ NOT NULL
    DEFAULT date_trunc('minute', now()) - interval '10 minutes';
