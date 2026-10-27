-- Metrics storage for per-organization retention (docs/adr/0001-metrics-storage.md).
--
-- The collector's exporter writes to the metrics_ingest_* tables (Null engine: nothing is stored
-- there). Materialized views normalize each insert into metrics_points (Null too) and fan it out to:
--   metrics_series     one row per series: its labels, type and unit (labels are stored once, not per point)
--   metrics_samples    (series, time, value) per point
--   metrics_rollup_5m  min / max / sum / count / first / last (and their times) per series and 5 minutes
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
    -- First and last sample time: rates divide by the time the samples span, as on raw samples.
    `Start` SimpleAggregateFunction(min, DateTime64(3)) CODEC(Delta, ZSTD(1)),
    `End` SimpleAggregateFunction(max, DateTime64(3)) CODEC(Delta, ZSTD(1)),
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
    min(TimeUnix) AS Start,
    max(TimeUnix) AS End,
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
    -- First and last sample time: rates divide by the time the samples span, as on raw samples.
    `Start` SimpleAggregateFunction(min, DateTime64(3)) CODEC(Delta, ZSTD(1)),
    `End` SimpleAggregateFunction(max, DateTime64(3)) CODEC(Delta, ZSTD(1)),
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
    min(TimeUnix) AS Start,
    max(TimeUnix) AS End,
    Retention1hDays AS RetentionDays
FROM otel.metrics_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId, Bucket, RetentionDays;

-- ================================================================ distributions
-- Histograms, exponential histograms and summaries follow the same model: one normalized point
-- table, then series, samples and rollups. A rollup keeps, per bucket of time, the first and last
-- cumulative value (a cumulative series' change is last − first), the total (a delta series' sum),
-- min / max and the bucket layout, so every statistic of the raw points can be computed from it.

CREATE TABLE IF NOT EXISTS otel.metrics_dist_points${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `MetricType` Enum8('gauge' = 1, 'sum' = 2, 'histogram' = 3, 'exponential_histogram' = 4, 'summary' = 5),
    `ServiceName` LowCardinality(String),
    `MetricUnit` LowCardinality(String),
    `MetricDescription` String,
    `AggregationTemporality` Int32,
    `ResourceAttributes` Map(LowCardinality(String), String),
    `Attributes` Map(LowCardinality(String), String),
    `TimeUnix` DateTime64(3),
    `Count` UInt64,
    `Sum` Float64,
    `Min` Float64,
    `Max` Float64,
    `BucketCounts` Array(UInt64),
    `ExplicitBounds` Array(Float64),
    `Scale` Int32,
    `ZeroCount` UInt64,
    `PositiveOffset` Int32,
    `PositiveBucketCounts` Array(UInt64),
    `NegativeOffset` Int32,
    `NegativeBucketCounts` Array(UInt64),
    `Quantiles` Array(Float64),
    `QuantileValues` Array(Float64),
    `RetentionDays` UInt16,
    `Retention5mDays` UInt16,
    `Retention1hDays` UInt16
)
ENGINE = Null;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_points_histogram_mv ${ON_CLUSTER} TO otel.metrics_dist_points${LOCAL} AS
-- src.* names the inserted columns: the output aliases below shadow them.
WITH mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), src.ResourceAttributes)) AS resource, mapSort(src.Attributes) AS labels
SELECT
    src.ResourceAttributes['oc.org_id'] AS OrgId, MetricName, cityHash64(OrgId, MetricName, MetricType, resource, labels) AS SeriesId, 'histogram' AS MetricType,
    ServiceName, MetricUnit, MetricDescription, AggregationTemporality, resource AS ResourceAttributes, labels AS Attributes, TimeUnix,
    Count, Sum, Min, Max, BucketCounts, ExplicitBounds, 0 AS Scale, toUInt64(0) AS ZeroCount, 0 AS PositiveOffset, CAST([], 'Array(UInt64)') AS PositiveBucketCounts,
    0 AS NegativeOffset, CAST([], 'Array(UInt64)') AS NegativeBucketCounts, CAST([], 'Array(Float64)') AS Quantiles, CAST([], 'Array(Float64)') AS QuantileValues,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_days']) AS RetentionDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_5m_days']) AS Retention5mDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays
FROM otel.metrics_ingest_histogram${LOCAL} AS src;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_points_exponential_histogram_mv ${ON_CLUSTER} TO otel.metrics_dist_points${LOCAL} AS
-- src.* names the inserted columns: the output aliases below shadow them.
WITH mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), src.ResourceAttributes)) AS resource, mapSort(src.Attributes) AS labels
SELECT
    src.ResourceAttributes['oc.org_id'] AS OrgId, MetricName, cityHash64(OrgId, MetricName, MetricType, resource, labels) AS SeriesId, 'exponential_histogram' AS MetricType,
    ServiceName, MetricUnit, MetricDescription, AggregationTemporality, resource AS ResourceAttributes, labels AS Attributes, TimeUnix,
    Count, Sum, Min, Max, CAST([], 'Array(UInt64)') AS BucketCounts, CAST([], 'Array(Float64)') AS ExplicitBounds, Scale, ZeroCount, PositiveOffset, PositiveBucketCounts,
    NegativeOffset, NegativeBucketCounts, CAST([], 'Array(Float64)') AS Quantiles, CAST([], 'Array(Float64)') AS QuantileValues,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_days']) AS RetentionDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_5m_days']) AS Retention5mDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays
FROM otel.metrics_ingest_exponential_histogram${LOCAL} AS src;

-- Summaries are cumulative and have no min / max (the 0 and 1 quantiles, when reported).
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_points_summary_mv ${ON_CLUSTER} TO otel.metrics_dist_points${LOCAL} AS
-- src.* names the inserted columns: the output aliases below shadow them.
WITH mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), src.ResourceAttributes)) AS resource, mapSort(src.Attributes) AS labels
SELECT
    src.ResourceAttributes['oc.org_id'] AS OrgId, MetricName, cityHash64(OrgId, MetricName, MetricType, resource, labels) AS SeriesId, 'summary' AS MetricType,
    ServiceName, MetricUnit, MetricDescription, 2 AS AggregationTemporality, resource AS ResourceAttributes, labels AS Attributes, TimeUnix,
    Count, Sum, 0. AS Min, 0. AS Max, CAST([], 'Array(UInt64)') AS BucketCounts, CAST([], 'Array(Float64)') AS ExplicitBounds, 0 AS Scale, toUInt64(0) AS ZeroCount,
    0 AS PositiveOffset, CAST([], 'Array(UInt64)') AS PositiveBucketCounts, 0 AS NegativeOffset, CAST([], 'Array(UInt64)') AS NegativeBucketCounts,
    `ValueAtQuantiles.Quantile` AS Quantiles, `ValueAtQuantiles.Value` AS QuantileValues,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_days']) AS RetentionDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_5m_days']) AS Retention5mDays,
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays
FROM otel.metrics_ingest_summary${LOCAL} AS src;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_series_mv ${ON_CLUSTER} TO otel.metrics_series${LOCAL} AS
SELECT
    OrgId,
    MetricName,
    SeriesId,
    any(MetricType) AS MetricType,
    any(ServiceName) AS ServiceName,
    any(MetricUnit) AS MetricUnit,
    any(MetricDescription) AS MetricDescription,
    any(AggregationTemporality) AS AggregationTemporality,
    false AS IsMonotonic,
    any(ResourceAttributes) AS ResourceAttributes,
    any(Attributes) AS Attributes,
    toDateTime(max(TimeUnix)) AS LastSeen,
    if(max(RetentionDays) = 0 OR max(Retention5mDays) = 0 OR max(Retention1hDays) = 0, 0,
       greatest(max(RetentionDays), max(Retention5mDays), max(Retention1hDays))) AS RetentionDays
FROM otel.metrics_dist_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId;

-- Column names as in the exporter's tables, so raw queries read them the same way.
CREATE TABLE IF NOT EXISTS otel.metrics_dist_samples${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `TimeUnix` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `Count` UInt64 CODEC(Delta, ZSTD(1)),
    `Sum` Float64 CODEC(Gorilla, ZSTD(1)),
    `Min` Float64 CODEC(Gorilla, ZSTD(1)),
    `Max` Float64 CODEC(Gorilla, ZSTD(1)),
    `BucketCounts` Array(UInt64) CODEC(ZSTD(1)),
    `ExplicitBounds` Array(Float64) CODEC(ZSTD(1)),
    `Scale` Int32 CODEC(ZSTD(1)),
    `ZeroCount` UInt64 CODEC(ZSTD(1)),
    `PositiveOffset` Int32 CODEC(ZSTD(1)),
    `PositiveBucketCounts` Array(UInt64) CODEC(ZSTD(1)),
    `NegativeOffset` Int32 CODEC(ZSTD(1)),
    `NegativeBucketCounts` Array(UInt64) CODEC(ZSTD(1)),
    `ValueAtQuantiles.Quantile` Array(Float64) CODEC(ZSTD(1)),
    `ValueAtQuantiles.Value` Array(Float64) CODEC(ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY (toDate(TimeUnix), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, TimeUnix)
TTL toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, 30, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_dist_samples SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_samples_mv ${ON_CLUSTER} TO otel.metrics_dist_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Count, Sum, Min, Max, BucketCounts, ExplicitBounds, Scale, ZeroCount, PositiveOffset, PositiveBucketCounts,
       NegativeOffset, NegativeBucketCounts, Quantiles AS `ValueAtQuantiles.Quantile`, QuantileValues AS `ValueAtQuantiles.Value`, RetentionDays
FROM otel.metrics_dist_points${LOCAL};

CREATE TABLE IF NOT EXISTS otel.metrics_dist_rollup_5m${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `CountFirst` AggregateFunction(argMin, UInt64, DateTime64(3)),
    `CountLast` AggregateFunction(argMax, UInt64, DateTime64(3)),
    `CountTotal` SimpleAggregateFunction(sum, UInt64),
    `SumFirst` AggregateFunction(argMin, Float64, DateTime64(3)),
    `SumLast` AggregateFunction(argMax, Float64, DateTime64(3)),
    `SumTotal` SimpleAggregateFunction(sum, Float64),
    `Min` SimpleAggregateFunction(min, Float64),
    `Max` SimpleAggregateFunction(max, Float64),
    -- Histogram buckets, or an exponential histogram's positive buckets.
    `BucketsFirst` AggregateFunction(argMin, Array(UInt64), DateTime64(3)),
    `BucketsLast` AggregateFunction(argMax, Array(UInt64), DateTime64(3)),
    `BucketsTotal` AggregateFunction(sumForEach, Array(UInt64)),
    `ExplicitBounds` SimpleAggregateFunction(anyLast, Array(Float64)),
    `Scale` SimpleAggregateFunction(anyLast, Int32),
    `PositiveOffset` SimpleAggregateFunction(anyLast, Int32),
    -- Summaries: the reported quantiles and the sum of their values over the bucket's points.
    `Quantiles` SimpleAggregateFunction(anyLast, Array(Float64)),
    `QuantileTotals` AggregateFunction(sumForEach, Array(Float64)),
    `Points` SimpleAggregateFunction(sum, UInt64),
    `Start` SimpleAggregateFunction(min, DateTime64(3)) CODEC(Delta, ZSTD(1)),
    `End` SimpleAggregateFunction(max, DateTime64(3)) CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toDate(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 90, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_dist_rollup_5m SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_rollup_5m_mv ${ON_CLUSTER} TO otel.metrics_dist_rollup_5m${LOCAL} AS
SELECT
    OrgId, MetricName, SeriesId, toStartOfFiveMinutes(TimeUnix) AS Bucket,
    argMinState(Count, TimeUnix) AS CountFirst, argMaxState(Count, TimeUnix) AS CountLast, sum(Count) AS CountTotal,
    argMinState(Sum, TimeUnix) AS SumFirst, argMaxState(Sum, TimeUnix) AS SumLast, sum(Sum) AS SumTotal,
    min(Min) AS Min, max(Max) AS Max,
    argMinState(if(MetricType = 'histogram', BucketCounts, PositiveBucketCounts), TimeUnix) AS BucketsFirst,
    argMaxState(if(MetricType = 'histogram', BucketCounts, PositiveBucketCounts), TimeUnix) AS BucketsLast,
    sumForEachState(if(MetricType = 'histogram', BucketCounts, PositiveBucketCounts)) AS BucketsTotal,
    anyLast(ExplicitBounds) AS ExplicitBounds, anyLast(Scale) AS Scale, anyLast(PositiveOffset) AS PositiveOffset,
    anyLast(Quantiles) AS Quantiles, sumForEachState(QuantileValues) AS QuantileTotals,
    count() AS Points, min(TimeUnix) AS Start, max(TimeUnix) AS End, Retention5mDays AS RetentionDays
FROM otel.metrics_dist_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId, Bucket, RetentionDays;

-- Monthly partitions, as metrics_rollup_1h.
CREATE TABLE IF NOT EXISTS otel.metrics_dist_rollup_1h${LOCAL} ${ON_CLUSTER} AS otel.metrics_dist_rollup_5m${LOCAL}
ENGINE = ${REPLICATED}AggregatingMergeTree
PARTITION BY (toYYYYMM(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_dist_rollup_1h SeriesId};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_rollup_1h_mv ${ON_CLUSTER} TO otel.metrics_dist_rollup_1h${LOCAL} AS
SELECT
    OrgId, MetricName, SeriesId, toStartOfHour(TimeUnix) AS Bucket,
    argMinState(Count, TimeUnix) AS CountFirst, argMaxState(Count, TimeUnix) AS CountLast, sum(Count) AS CountTotal,
    argMinState(Sum, TimeUnix) AS SumFirst, argMaxState(Sum, TimeUnix) AS SumLast, sum(Sum) AS SumTotal,
    min(Min) AS Min, max(Max) AS Max,
    argMinState(if(MetricType = 'histogram', BucketCounts, PositiveBucketCounts), TimeUnix) AS BucketsFirst,
    argMaxState(if(MetricType = 'histogram', BucketCounts, PositiveBucketCounts), TimeUnix) AS BucketsLast,
    sumForEachState(if(MetricType = 'histogram', BucketCounts, PositiveBucketCounts)) AS BucketsTotal,
    anyLast(ExplicitBounds) AS ExplicitBounds, anyLast(Scale) AS Scale, anyLast(PositiveOffset) AS PositiveOffset,
    anyLast(Quantiles) AS Quantiles, sumForEachState(QuantileValues) AS QuantileTotals,
    count() AS Points, min(TimeUnix) AS Start, max(TimeUnix) AS End, Retention1hDays AS RetentionDays
FROM otel.metrics_dist_points${LOCAL}
GROUP BY OrgId, MetricName, SeriesId, Bucket, RetentionDays;
