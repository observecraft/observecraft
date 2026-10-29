-- Tag index for metrics (docs/adr/0002-storage-at-saas-scale.md, stage 3).
--
-- metrics_tags: one row per series and label (key, value), so a filter like
-- kube_namespace:billing is a primary-key range instead of an expression over the label maps of
-- every series; queries then read samples of `MetricName = ? AND SeriesId IN (…)`. Fed by a view
-- from the series rows (about one per series and hour). Scope says where the value comes from,
-- as bits (several when they agree):
--   1  a data-point attribute (Attributes)
--   2  a resource attribute (ResourceAttributes; Datadog's short tags such as kube_namespace read
--      these, under their OpenTelemetry keys such as k8s.namespace.name)
--   4  the service (ServiceName, under the key `service`)
--   8  the value a plain key reads: the data-point attribute, else the resource attribute
-- Sharded by series like the series and samples, so series → samples joins stay on the shard.
--
-- metrics_tag_values: the values of every key (with the same Scope bits), for template variables
-- and tag pickers: one small range read per key. Sharded by (org, key).
--
-- LastSeen is the series rows' (a time in every hour the series reported); existing series get
-- their rows once below.

CREATE TABLE IF NOT EXISTS otel.metrics_tags${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `Key` LowCardinality(String),
    `Value` String CODEC(ZSTD(1)),
    `SeriesId` UInt64,
    `MetricType` Enum8('gauge' = 1, 'sum' = 2, 'histogram' = 3, 'exponential_histogram' = 4, 'summary' = 5),
    `Scope` UInt8,
    `LastSeen` DateTime CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree(LastSeen)
PARTITION BY (toYYYYMM(LastSeen), RetentionDays)
ORDER BY (OrgId, MetricName, Key, Value, SeriesId)
TTL LastSeen + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_tags SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_tags_mv ${ON_CLUSTER} TO otel.metrics_tags${LOCAL} AS
SELECT OrgId, MetricName, tupleElement(tag, 1) AS Key, tupleElement(tag, 2) AS Value, SeriesId, any(MetricType) AS MetricType,
       groupBitOr(tupleElement(tag, 3)) AS Scope, max(LastSeen) AS LastSeen, max(RetentionDays) AS RetentionDays
FROM otel.metrics_series${LOCAL}
ARRAY JOIN arrayConcat(
    arrayMap((k, v) -> (toString(k), v, toUInt8(if(mapContains(Attributes, k), 2, 10))), mapKeys(ResourceAttributes), mapValues(ResourceAttributes)),
    arrayMap((k, v) -> (toString(k), v, toUInt8(9)), mapKeys(Attributes), mapValues(Attributes)),
    if(ServiceName = '', [], [('service', toString(ServiceName), toUInt8(4))])
) AS tag
GROUP BY OrgId, MetricName, SeriesId, Key, Value;

CREATE TABLE IF NOT EXISTS otel.metrics_tag_values${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Key` LowCardinality(String),
    `Value` String CODEC(ZSTD(1)),
    `Scope` UInt8,
    `LastSeen` DateTime CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree(LastSeen)
PARTITION BY (toYYYYMM(LastSeen), RetentionDays)
ORDER BY (OrgId, Key, Value, Scope)
TTL LastSeen + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_tag_values cityHash64(OrgId, Key)};

-- Into the Distributed table in a cluster: values are sharded by key, not by series.
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_tag_values_mv ${ON_CLUSTER} TO otel.metrics_tag_values AS
SELECT OrgId, Key, Value, Scope, max(LastSeen) AS LastSeen, max(RetentionDays) AS RetentionDays
FROM otel.metrics_tags${LOCAL}
GROUP BY OrgId, Key, Value, Scope;

-- The series already stored.
INSERT INTO otel.metrics_tags (OrgId, MetricName, Key, Value, SeriesId, MetricType, Scope, LastSeen, RetentionDays)
SELECT OrgId, MetricName, tupleElement(tag, 1) AS Key, tupleElement(tag, 2) AS Value, SeriesId, any(MetricType) AS MetricType,
       groupBitOr(tupleElement(tag, 3)) AS Scope, max(LastSeen) AS LastSeen, max(RetentionDays) AS RetentionDays
FROM otel.metrics_series
ARRAY JOIN arrayConcat(
    arrayMap((k, v) -> (toString(k), v, toUInt8(if(mapContains(Attributes, k), 2, 10))), mapKeys(ResourceAttributes), mapValues(ResourceAttributes)),
    arrayMap((k, v) -> (toString(k), v, toUInt8(9)), mapKeys(Attributes), mapValues(Attributes)),
    if(ServiceName = '', [], [('service', toString(ServiceName), toUInt8(4))])
) AS tag
GROUP BY OrgId, MetricName, SeriesId, Key, Value
