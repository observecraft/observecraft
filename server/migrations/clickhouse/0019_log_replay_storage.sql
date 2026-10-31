-- New writes use replay-safe tables. Legacy tables remain for compatibility and their writes are
-- mirrored before backfill. Backfills are retry-safe: row reads and aggregate states deduplicate IDs.
-- Identity is (org_id, id) with immutable normalized fields. Shard by that identity so retries
-- cannot land on another shard. FINAL is applied in the read views, not left to background merges.
-- Exact aggregate states are finalized on each shard before counts cross the network. Ordered
-- aggregation limits the state held while reading many minutes; shard identity makes counts additive.
CREATE TABLE IF NOT EXISTS oc.logs_dedup${LOCAL} ${ON_CLUSTER} AS oc.logs${LOCAL}
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toDate(timestamp), retention_days)
ORDER BY (org_id, service, timestamp, id)
TTL toDateTime(timestamp) + toIntervalDay(retention_days)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED oc.logs_dedup cityHash64(org_id, id)};

CREATE TABLE IF NOT EXISTS oc.logs_raw_dedup${LOCAL} ${ON_CLUSTER} AS oc.logs_raw${LOCAL}
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY toDate(timestamp)
ORDER BY (org_id, service, timestamp, id)
TTL toDateTime(timestamp) + INTERVAL 72 HOUR
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED oc.logs_raw_dedup cityHash64(org_id, id)};
-- Raw retention is installation-specific. Use the maximum supported raw retention during backfill
-- (Config.raw_logs_hours is capped at 30 days), rather than expiring copied parts after 72 hours.
-- MODIFY is retry-safe, including an interrupted backfill; startup then applies the configured TTL.
ALTER TABLE oc.logs_raw_dedup${LOCAL} ${ON_CLUSTER}
    MODIFY TTL toDateTime(timestamp) + INTERVAL 30 DAY;

CREATE VIEW IF NOT EXISTS oc.logs_read ${ON_CLUSTER} AS
SELECT *, error_kind, error_key FROM oc.logs_dedup FINAL;
CREATE VIEW IF NOT EXISTS oc.logs_raw_read ${ON_CLUSTER} AS
SELECT * FROM oc.logs_raw_dedup FINAL;

CREATE TABLE IF NOT EXISTS oc.error_groups_1m_dedup${LOCAL} ${ON_CLUSTER}
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
    events         AggregateFunction(uniqExact, UUID),
    first_seen      SimpleAggregateFunction(min, DateTime64(3, 'UTC')),
    last_seen       SimpleAggregateFunction(max, DateTime64(3, 'UTC')),
    -- The newest message (first 300 characters).
    sample          AggregateFunction(argMax, String, DateTime64(3, 'UTC')) CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toDate(minute), retention_days)
ORDER BY (org_id, minute, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace)
TTL minute + toIntervalDay(retention_days);
${DISTRIBUTED oc.error_groups_1m_dedup rand()};

CREATE MATERIALIZED VIEW IF NOT EXISTS oc.error_groups_1m_dedup_mv ${ON_CLUSTER} TO oc.error_groups_1m_dedup${LOCAL} AS
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
WHERE status = 'error'
GROUP BY org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace;

CREATE VIEW IF NOT EXISTS oc.error_groups_1m_read${LOCAL} ${ON_CLUSTER} AS
SELECT org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace, uniqExactMerge(events) AS n, min(first_seen) AS first_seen, max(last_seen) AS last_seen, argMaxMergeState(sample) AS sample
FROM oc.error_groups_1m_dedup${LOCAL} GROUP BY org_id, minute, retention_days, service, error_kind, error_key, index_name, host, source, env, version, kube_namespace SETTINGS optimize_aggregation_in_order = 1;
${DISTRIBUTED oc.error_groups_1m_read rand()};

CREATE TABLE IF NOT EXISTS oc.log_facets_1m_dedup${LOCAL} ${ON_CLUSTER}
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
    events             AggregateFunction(uniqExact, UUID)
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toDate(minute), retention_days)
ORDER BY (org_id, minute, index_name, service, status, host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name)
TTL minute + toIntervalDay(retention_days);
${DISTRIBUTED oc.log_facets_1m_dedup rand()};

CREATE MATERIALIZED VIEW IF NOT EXISTS oc.log_facets_1m_dedup_mv ${ON_CLUSTER} TO oc.log_facets_1m_dedup${LOCAL} AS
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
    uniqExactState(id) AS events
FROM oc.logs_dedup${LOCAL}
GROUP BY org_id, minute, retention_days, index_name, service, status, host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name;

CREATE VIEW IF NOT EXISTS oc.log_facets_1m_read${LOCAL} ${ON_CLUSTER} AS
SELECT org_id, minute, retention_days, index_name, service, status, host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name, uniqExactMerge(events) AS n
FROM oc.log_facets_1m_dedup${LOCAL} GROUP BY org_id, minute, retention_days, index_name, service, status, host, source, env, kube_cluster_name, region, kube_namespace, kube_deployment, container_name SETTINGS optimize_aggregation_in_order = 1;
${DISTRIBUTED oc.log_facets_1m_read rand()};

-- Install forwarding before copying historical rows; a restarted backfill may repeat rows safely.
-- Pause ingestion and drain old consumers before this upgrade, then resume after migration finishes.
CREATE MATERIALIZED VIEW IF NOT EXISTS oc.logs_legacy_to_dedup_mv ${ON_CLUSTER}
TO oc.logs_dedup AS SELECT * FROM oc.logs${LOCAL};
CREATE MATERIALIZED VIEW IF NOT EXISTS oc.logs_raw_legacy_to_dedup_mv ${ON_CLUSTER}
TO oc.logs_raw_dedup AS SELECT * FROM oc.logs_raw${LOCAL};
INSERT INTO oc.logs_dedup SELECT * FROM oc.logs SETTINGS insert_distributed_sync = 1;
INSERT INTO oc.logs_raw_dedup SELECT * FROM oc.logs_raw SETTINGS insert_distributed_sync = 1;
