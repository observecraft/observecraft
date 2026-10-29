-- Log aggregates maintained at ingest (ADR-0002), so the Errors page and the standard log facets read a
-- few rows per minute instead of every log in range. Views over the shard-local indexed logs fill
-- them per insert block; rows of the same minute and dimensions merge in the background, and readers
-- still GROUP BY. Each row carries its logs' retention: partitions split by it, so rows expire with
-- the logs they count. The aggregates start empty: readers use them from their first complete
-- minute of an org on and read older logs from oc.logs.

-- Error Tracking's issues: error logs per minute and issue key (service, error_kind, error_key; see
-- 0007_log_error_keys), with the dimensions an issue search and its facets filter on.
CREATE TABLE IF NOT EXISTS oc.error_groups_1m${LOCAL} ${ON_CLUSTER}
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
    n               SimpleAggregateFunction(sum, UInt64),
    first_seen      SimpleAggregateFunction(min, DateTime64(3, 'UTC')),
    last_seen       SimpleAggregateFunction(max, DateTime64(3, 'UTC')),
    -- The newest message (first 300 characters).
    sample          AggregateFunction(argMax, String, DateTime64(3, 'UTC')) CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toDate(minute), retention_days)
ORDER BY (org_id, minute, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace)
TTL minute + toIntervalDay(retention_days);
${DISTRIBUTED oc.error_groups_1m rand()};

CREATE MATERIALIZED VIEW IF NOT EXISTS oc.error_groups_1m_mv ${ON_CLUSTER} TO oc.error_groups_1m${LOCAL} AS
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
    count() AS n,
    min(timestamp) AS first_seen,
    max(timestamp) AS last_seen,
    argMaxState(substringUTF8(message, 1, 300), timestamp) AS sample
FROM oc.logs${LOCAL}
WHERE status = 'error'
GROUP BY org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace;

-- Log counts per minute and combination of the facets the Log Explorer shows by default (high-
-- cardinality ones such as pod_name, and attributes, stay out: their counts read the logs).
CREATE TABLE IF NOT EXISTS oc.log_facets_1m${LOCAL} ${ON_CLUSTER}
(
    org_id             LowCardinality(String),
    minute             DateTime('UTC'),
    retention_days     UInt16,
    index_name         LowCardinality(String),
    service            LowCardinality(String),
    status             LowCardinality(String),
    host               LowCardinality(String),
    source             LowCardinality(String),
    env                LowCardinality(String),
    kube_cluster_name  LowCardinality(String),
    region             LowCardinality(String),
    kube_namespace     LowCardinality(String),
    kube_deployment    LowCardinality(String),
    container_name     LowCardinality(String),
    n                  UInt64
)
ENGINE = ${REPLICATED}SummingMergeTree(n)
PARTITION BY (toDate(minute), retention_days)
ORDER BY (org_id, minute, index_name, service, status, host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name)
TTL minute + toIntervalDay(retention_days);
${DISTRIBUTED oc.log_facets_1m rand()};

CREATE MATERIALIZED VIEW IF NOT EXISTS oc.log_facets_1m_mv ${ON_CLUSTER} TO oc.log_facets_1m${LOCAL} AS
SELECT
    org_id,
    toStartOfMinute(timestamp) AS minute,
    retention_days,
    index_name,
    service,
    status,
    host,
    source,
    tags['env'] AS env,
    tags['kube_cluster_name'] AS kube_cluster_name,
    tags['region'] AS region,
    tags['kube_namespace'] AS kube_namespace,
    tags['kube_deployment'] AS kube_deployment,
    tags['container_name'] AS container_name,
    count() AS n
FROM oc.logs${LOCAL}
GROUP BY org_id, minute, retention_days, index_name, service, status, host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name;
