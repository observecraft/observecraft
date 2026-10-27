-- Metrics storage for per-organization retention (docs/adr/0001-metrics-storage.md).
--
-- The collector's exporter writes to the metrics_ingest_* tables (Null engine: nothing is stored
-- there). Materialized views normalize each insert into metrics_points (Null too) and fan it out to:
--   metrics_series     one row per series: its labels, type and unit (labels are stored once, not per point)
--   metrics_samples    (series, time, value) per point
--   metrics_rollup_5m  min / max / sum / count / first / last per series and 5 minutes
--   metrics_rollup_1h  the same per hour
-- A series is identified by its org, metric name, type and sorted labels, so the order a sender uses
-- doesn't matter; the organization's own oc.* attributes (org, retention) are not labels. In a
-- cluster the ingest tables shard by that id, so a series, its samples and rollups share a shard.
--
-- Each row carries its org's retention for its table (oc.metrics_*_days resource attributes the
-- intake stamps, 0 = the install default). Retention is part of the partition key, so expired data
-- is dropped as whole partitions. The defaults below are replaced at startup (oc_store::apply_retention).

-- ================================================================ ingest (what the exporter writes)

CREATE TABLE IF NOT EXISTS otel.metrics_ingest_gauge${LOCAL} ${ON_CLUSTER} AS otel.otel_metrics_gauge${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_ingest_gauge cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

CREATE TABLE IF NOT EXISTS otel.metrics_ingest_sum${LOCAL} ${ON_CLUSTER} AS otel.otel_metrics_sum${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_ingest_sum cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

-- Distributions keep their current tables until they move to this model; the exporter needs a
-- table for every type.
CREATE TABLE IF NOT EXISTS otel.metrics_ingest_histogram${LOCAL} ${ON_CLUSTER} AS otel.otel_metrics_histogram${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_ingest_histogram cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

CREATE TABLE IF NOT EXISTS otel.metrics_ingest_exponential_histogram${LOCAL} ${ON_CLUSTER} AS otel.otel_metrics_exponential_histogram${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_ingest_exponential_histogram cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

CREATE TABLE IF NOT EXISTS otel.metrics_ingest_summary${LOCAL} ${ON_CLUSTER} AS otel.otel_metrics_summary${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_ingest_summary cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

-- ================================================================ normalized points

CREATE TABLE IF NOT EXISTS otel.metrics_points${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `MetricType` Enum8('gauge' = 1, 'sum' = 2, 'histogram' = 3, 'exponential_histogram' = 4, 'summary' = 5),
    `ServiceName` LowCardinality(String),
    `MetricUnit` LowCardinality(String),
    `MetricDescription` String,
    `AggregationTemporality` Int32,
    `IsMonotonic` Bool,
    `ResourceAttributes` Map(LowCardinality(String), String),
    `Attributes` Map(LowCardinality(String), String),
    `TimeUnix` DateTime64(3),
    `Value` Float64,
    `RetentionDays` UInt16,
    `Retention5mDays` UInt16,
    `Retention1hDays` UInt16
)
ENGINE = Null;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_points_gauge_mv ${ON_CLUSTER} TO otel.metrics_points${LOCAL} AS
-- src.* names the inserted columns: the output aliases below shadow them.
WITH mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), src.ResourceAttributes)) AS resource, mapSort(src.Attributes) AS labels
SELECT
    src.ResourceAttributes['oc.org_id'] AS OrgId,
    MetricName,
    cityHash64(OrgId, MetricName, MetricType, resource, labels) AS SeriesId,
    'gauge' AS MetricType,
    ServiceName,
    MetricUnit,
    MetricDescription,
    0 AS AggregationTemporality,
    false AS IsMonotonic,
    resource AS ResourceAttributes,
    labels AS Attributes,
    TimeUnix,
    Value,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_days']) AS RetentionDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_5m_days']) AS Retention5mDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays
FROM otel.metrics_ingest_gauge${LOCAL} AS src;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_points_sum_mv ${ON_CLUSTER} TO otel.metrics_points${LOCAL} AS
-- src.* names the inserted columns: the output aliases below shadow them.
WITH mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), src.ResourceAttributes)) AS resource, mapSort(src.Attributes) AS labels
SELECT
    src.ResourceAttributes['oc.org_id'] AS OrgId,
    MetricName,
    cityHash64(OrgId, MetricName, MetricType, resource, labels) AS SeriesId,
    'sum' AS MetricType,
    ServiceName,
    MetricUnit,
    MetricDescription,
    AggregationTemporality,
    IsMonotonic,
    resource AS ResourceAttributes,
    labels AS Attributes,
    TimeUnix,
    Value,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_days']) AS RetentionDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_5m_days']) AS Retention5mDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays
FROM otel.metrics_ingest_sum${LOCAL} AS src;

-- ================================================================ series

-- One row per series and insert; merges keep the latest. Readers dedupe by SeriesId (FINAL or argMax).
CREATE TABLE IF NOT EXISTS otel.metrics_series${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `MetricType` Enum8('gauge' = 1, 'sum' = 2, 'histogram' = 3, 'exponential_histogram' = 4, 'summary' = 5),
    `ServiceName` LowCardinality(String),
    `MetricUnit` LowCardinality(String),
    `MetricDescription` String CODEC(ZSTD(1)),
    `AggregationTemporality` Int32,
    `IsMonotonic` Bool,
    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `Attributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `LastSeen` DateTime CODEC(Delta, ZSTD(1)),
    -- The longest of the org's metric retentions: a series is kept as long as any of its data.
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree(LastSeen)
PARTITION BY (toYYYYMM(LastSeen), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId)
TTL LastSeen + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_series SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_series_mv ${ON_CLUSTER} TO otel.metrics_series${LOCAL} AS
SELECT
    OrgId,
    MetricName,
    SeriesId,
    any(MetricType) AS MetricType,
    any(ServiceName) AS ServiceName,
    any(MetricUnit) AS MetricUnit,
    any(MetricDescription) AS MetricDescription,
    any(AggregationTemporality) AS AggregationTemporality,
    any(IsMonotonic) AS IsMonotonic,
    any(ResourceAttributes) AS ResourceAttributes,
    any(Attributes) AS Attributes,
    toDateTime(max(TimeUnix)) AS LastSeen,
    if(max(RetentionDays) = 0 OR max(Retention5mDays) = 0 OR max(Retention1hDays) = 0, 0,
       greatest(max(RetentionDays), max(Retention5mDays), max(Retention1hDays))) AS RetentionDays
FROM otel.metrics_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId;

-- ================================================================ samples

CREATE TABLE IF NOT EXISTS otel.metrics_samples${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `TimeUnix` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `Value` Float64 CODEC(Gorilla, ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY (toDate(TimeUnix), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, TimeUnix)
TTL toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, 30, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_samples SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_samples_mv ${ON_CLUSTER} TO otel.metrics_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Value, RetentionDays
FROM otel.metrics_points${LOCAL};

-- ================================================================ rollups

CREATE TABLE IF NOT EXISTS otel.metrics_rollup_5m${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `Min` SimpleAggregateFunction(min, Float64) CODEC(Gorilla, ZSTD(1)),
    `Max` SimpleAggregateFunction(max, Float64) CODEC(Gorilla, ZSTD(1)),
    `Sum` SimpleAggregateFunction(sum, Float64) CODEC(Gorilla, ZSTD(1)),
    `Count` SimpleAggregateFunction(sum, UInt64) CODEC(T64, ZSTD(1)),
    `First` AggregateFunction(argMin, Float64, DateTime64(3)),
    `Last` AggregateFunction(argMax, Float64, DateTime64(3)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toDate(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 90, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_rollup_5m SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_rollup_5m_mv ${ON_CLUSTER} TO otel.metrics_rollup_5m${LOCAL} AS
SELECT
    OrgId,
    MetricName,
    SeriesId,
    toStartOfFiveMinutes(TimeUnix) AS Bucket,
    min(Value) AS Min,
    max(Value) AS Max,
    sum(Value) AS Sum,
    count() AS Count,
    argMinState(Value, TimeUnix) AS First,
    argMaxState(Value, TimeUnix) AS Last,
    Retention5mDays AS RetentionDays
FROM otel.metrics_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId, Bucket, RetentionDays;

-- Monthly partitions: a year is 12 partitions per retention, not 365.
CREATE TABLE IF NOT EXISTS otel.metrics_rollup_1h${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `Min` SimpleAggregateFunction(min, Float64) CODEC(Gorilla, ZSTD(1)),
    `Max` SimpleAggregateFunction(max, Float64) CODEC(Gorilla, ZSTD(1)),
    `Sum` SimpleAggregateFunction(sum, Float64) CODEC(Gorilla, ZSTD(1)),
    `Count` SimpleAggregateFunction(sum, UInt64) CODEC(T64, ZSTD(1)),
    `First` AggregateFunction(argMin, Float64, DateTime64(3)),
    `Last` AggregateFunction(argMax, Float64, DateTime64(3)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toYYYYMM(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_rollup_1h SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_rollup_1h_mv ${ON_CLUSTER} TO otel.metrics_rollup_1h${LOCAL} AS
SELECT
    OrgId,
    MetricName,
    SeriesId,
    toStartOfHour(TimeUnix) AS Bucket,
    min(Value) AS Min,
    max(Value) AS Max,
    sum(Value) AS Sum,
    count() AS Count,
    argMinState(Value, TimeUnix) AS First,
    argMaxState(Value, TimeUnix) AS Last,
    Retention1hDays AS RetentionDays
FROM otel.metrics_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId, Bucket, RetentionDays;
