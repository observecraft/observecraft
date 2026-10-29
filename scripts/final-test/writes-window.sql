-- Writes and CPU of a fixed window, normalized to one hour (storage-report.sql's measures, for a window that
-- ended before the page measurements started). Params: from, to ('YYYY-MM-DD hh:mm:ss', UTC).
--   curl -s 'localhost:18201/?param_from=...&param_to=...' --data-binary @scripts/final-test/writes-window.sql
SELECT * FROM (
SELECT 'write' AS what, database || '.' || table AS name,
       round(sum(rows) * 3600 / dateDiff('second', toDateTime({from:String}), toDateTime({to:String}))) AS rows_per_hour,
       round(sum(size_in_bytes) * 3600 / dateDiff('second', toDateTime({from:String}), toDateTime({to:String})) / 1048576, 1) AS mib_per_hour,
       count() AS parts
FROM system.part_log
WHERE event_type = 'NewPart' AND database IN ('otel', 'oc')
  AND event_time >= toDateTime({from:String}) AND event_time < toDateTime({to:String})
GROUP BY name
UNION ALL
SELECT 'merge', database || '.' || table,
       round(sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) / 1e6 * 3600 / dateDiff('second', toDateTime({from:String}), toDateTime({to:String})), 1),
       0, count()
FROM system.part_log
WHERE event_type = 'MergeParts'
  AND event_time >= toDateTime({from:String}) AND event_time < toDateTime({to:String})
GROUP BY database, table
UNION ALL
SELECT 'cpu', toString(query_kind),
       round(sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) / 1e6 * 3600 / dateDiff('second', toDateTime({from:String}), toDateTime({to:String})), 1),
       0, count()
FROM system.query_log
WHERE type IN ('QueryFinish', 'ExceptionWhileProcessing')
  AND event_time >= toDateTime({from:String}) AND event_time < toDateTime({to:String})
GROUP BY query_kind
UNION ALL
SELECT 'cpu', 'Merge (all tables)',
       round(sum(ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']) / 1e6 * 3600 / dateDiff('second', toDateTime({from:String}), toDateTime({to:String})), 1),
       0, count()
FROM system.part_log
WHERE event_type = 'MergeParts'
  AND event_time >= toDateTime({from:String}) AND event_time < toDateTime({to:String})

) ORDER BY what, rows_per_hour DESC
FORMAT TSV
