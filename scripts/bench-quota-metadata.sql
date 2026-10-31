-- Storage-only microbenchmark after migration 0058, e.g.:
--   psql "$OC_TEST_POSTGRES_URL" -v ON_ERROR_STOP=1 -f scripts/bench-quota-metadata.sql
-- Creates only a session-local table and rolls back. No application rows are changed.
-- Fresh ordered UUID inserts, one 36-character org, short index name, 512-event batches.
-- Excludes WAL, replication, vacuum/bloat and application/ClickHouse throughput.
BEGIN;
CREATE TEMP TABLE oc_quota_receipt_bench (LIKE log_quota_admissions INCLUDING ALL) ON COMMIT DROP;
INSERT INTO oc_quota_receipt_bench (org_id, event_id, index_name, retention_days, expires_at)
SELECT '00000000-0000-4000-8000-000000000001',
       ('00000000-0000-7000-8000-' || lpad(n::text, 12, '0'))::uuid,
       'main', 7, now() + INTERVAL '7 days' + (n / 512) * INTERVAL '1 millisecond'
FROM generate_series(1, 100000) AS n;
SELECT count(*) AS receipts,
       pg_total_relation_size('pg_temp.oc_quota_receipt_bench') AS heap_and_indexes_bytes,
       sum(pg_column_size(r)) AS tuple_bytes,
       round(pg_total_relation_size('pg_temp.oc_quota_receipt_bench') / count(*)::numeric, 1) AS bytes_per_receipt
FROM oc_quota_receipt_bench r;
ROLLBACK;
