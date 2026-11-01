-- Error Tracking issues from logs follow Datadog's rule: an error log (Vector maps critical, alert,
-- emergency and fatal to `error`) of a service with an error kind (error.kind / error.type /
-- exception.type: the error_kind column) or a stack trace (error.stack / exception.stacktrace).
-- Other error logs, such as an application's INFO lines on stderr that the Datadog Agent marks as
-- errors, stay errors in the Log Explorer but are no issues. The condition is
-- error_tracking::TRACKED_LOG_SQL (a test checks they are the same).
-- No backfill: before this aggregate's first minute, Error Tracking reads the logs with the same rule.
CREATE TABLE IF NOT EXISTS oc.error_issues_1m${LOCAL} ${ON_CLUSTER}
(
    org_id          LowCardinality(String),
    minute          DateTime('UTC'),
    retention_days  UInt16,
    service         LowCardinality(String),
    error_kind      LowCardinality(String),
    error_key       String CODEC(ZSTD(1)),
    index_name      LowCardinality(String),
    host            LowCardinality(String),
    source          LowCardinality(String),
    env             LowCardinality(String),
    version         LowCardinality(String),
    kube_namespace  LowCardinality(String),
    events          AggregateFunction(uniqExact, UUID),
    first_seen      SimpleAggregateFunction(min, DateTime64(3, 'UTC')),
    last_seen       SimpleAggregateFunction(max, DateTime64(3, 'UTC')),
    -- The newest message (first 300 characters).
    sample          AggregateFunction(argMax, String, DateTime64(3, 'UTC')) CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toDate(minute), retention_days)
ORDER BY (org_id, minute, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace)
TTL minute + toIntervalDay(retention_days);
${DISTRIBUTED oc.error_issues_1m rand()};

CREATE MATERIALIZED VIEW IF NOT EXISTS oc.error_issues_1m_mv ${ON_CLUSTER} TO oc.error_issues_1m${LOCAL} AS
SELECT
    org_id,
    toStartOfMinute(timestamp) AS minute,
    retention_days,
    service,
    error_kind,
    error_key,
    index_name,
    host,
    source,
    tags['env'] AS env,
    tags['version'] AS version,
    tags['kube_namespace'] AS kube_namespace,
    uniqExactState(id) AS events,
    min(timestamp) AS first_seen,
    max(timestamp) AS last_seen,
    argMaxState(substringUTF8(message, 1, 300), timestamp) AS sample
FROM oc.logs_dedup${LOCAL}
WHERE status = 'error' AND service != '' AND (error_kind != '' OR attr['error.stack'] != '' OR attr['exception.stacktrace'] != '')
GROUP BY org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace;

CREATE VIEW IF NOT EXISTS oc.error_issues_1m_read${LOCAL} ${ON_CLUSTER} AS
SELECT org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace, uniqExactMerge(events) AS n, min(first_seen) AS first_seen, max(last_seen) AS last_seen, argMaxMergeState(sample) AS sample
FROM oc.error_issues_1m${LOCAL} GROUP BY org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace SETTINGS optimize_aggregation_in_order = 1;
${DISTRIBUTED oc.error_issues_1m_read rand()};

-- The previous aggregate (every error log) is no longer read; stop filling it. Its table stays until
-- its rows expire, so an earlier server version can still be rolled back to.
DROP VIEW IF EXISTS oc.error_groups_1m_dedup_mv ${ON_CLUSTER};
