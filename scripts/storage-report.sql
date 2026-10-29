-- Storage report: what ClickHouse wrote, merged and spent CPU on in the last {window:UInt32} seconds
-- (docs/adr/0002, the baseline in IMPLEMENTATION_PLAN.md). Read from system.part_log and
-- system.query_log; `scripts/storage-report.mjs` (`make storage-report`) runs it and, given a cluster
-- name, reads every replica through clusterAllReplicas. By hand:
--
--   clickhouse client --param_window=3600 --multiquery < scripts/storage-report.sql
--
-- CPU is user + system time of the threads involved (UserTimeMicroseconds + SystemTimeMicroseconds).
-- Rates are normalized to one hour. Queries tagged log_comment = 'storage-report' (this report) are
-- left out. Aliases differ from column names: ClickHouse rejects `sum(rows) AS rows`.

-- Rows and bytes written (new parts) per table
SELECT
    database,
    table,
    count() AS new_parts,
    sum(rows) AS rows_written,
    round(sum(rows) * 3600 / {window:UInt32}) AS rows_per_hour,
    formatReadableSize(sum(size_in_bytes)) AS bytes_written,
    formatReadableSize(sum(size_in_bytes) * 3600 / {window:UInt32}) AS bytes_per_hour,
    formatReadableSize(sum(bytes_uncompressed)) AS uncompressed
FROM system.part_log
WHERE event_type = 'NewPart'
  AND event_time >= now() - toIntervalSecond({window:UInt32})
  AND database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema')
GROUP BY database, table
ORDER BY sum(rows) DESC;

-- Merges per table
SELECT
    database,
    table,
    count() AS merges,
    sum(rows) AS rows_merged,
    formatReadableSize(sum(size_in_bytes)) AS bytes_merged,
    round(sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) / 1e6, 1) AS cpu_seconds,
    round(sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) / 1e6 * 3600 / {window:UInt32}, 1) AS cpu_seconds_per_hour,
    round(sum(duration_ms) / 1e3, 1) AS wall_seconds
FROM system.part_log
WHERE event_type = 'MergeParts'
  AND event_time >= now() - toIntervalSecond({window:UInt32})
GROUP BY database, table
ORDER BY sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) DESC;

-- CPU by query kind and merges
SELECT
    kind,
    count() AS events,
    round(sum(cpu_us) / 1e6, 1) AS cpu_seconds,
    round(sum(cpu_us) / 1e6 * 3600 / {window:UInt32}, 1) AS cpu_seconds_per_hour,
    sum(read_rows_n) AS rows_read,
    formatReadableSize(sum(read_bytes_n)) AS bytes_read,
    sum(written_rows_n) AS rows_inserted
FROM
(
    SELECT
        toString(query_kind) AS kind,
        ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds'] AS cpu_us,
        read_rows AS read_rows_n,
        read_bytes AS read_bytes_n,
        written_rows AS written_rows_n
    FROM system.query_log
    WHERE type IN ('QueryFinish', 'ExceptionWhileProcessing')
      AND event_time >= now() - toIntervalSecond({window:UInt32})
      AND log_comment != 'storage-report'
    UNION ALL
    SELECT
        if(event_type = 'MergeParts', 'Merge', 'Mutation') AS kind,
        ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds'] AS cpu_us,
        0 AS read_rows_n,
        0 AS read_bytes_n,
        0 AS written_rows_n
    FROM system.part_log
    WHERE event_type IN ('MergeParts', 'MutatePart')
      AND event_time >= now() - toIntervalSecond({window:UInt32})
)
GROUP BY kind
ORDER BY sum(cpu_us) DESC;

-- Top 10 SELECT shapes by CPU
SELECT
    normalizedQueryHash(query) AS shape,
    count() AS runs,
    round(sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) / 1e6, 2) AS cpu_seconds,
    round(avg(query_duration_ms)) AS avg_ms,
    round(quantile(0.95)(query_duration_ms)) AS p95_ms,
    round(avg(read_rows)) AS avg_rows_read,
    formatReadableSize(avg(read_bytes)) AS avg_bytes_read,
    substring(normalizeQuery(any(query)), 1, 240) AS sample_query
FROM system.query_log
WHERE type = 'QueryFinish'
  AND query_kind = 'Select'
  AND is_initial_query
  AND event_time >= now() - toIntervalSecond({window:UInt32})
  AND log_comment != 'storage-report'
GROUP BY shape
ORDER BY sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) DESC
LIMIT 10;
