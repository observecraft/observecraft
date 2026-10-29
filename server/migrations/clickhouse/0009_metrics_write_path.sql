-- Metrics write path at SaaS scale (docs/adr/0002-storage-at-saas-scale.md, stage 2).
--
-- Materialized views run once per insert block, and a block holds about one point per series, so
-- views that aggregate per series wrote a row per point: a series row (both label maps) and two
-- rollup rows for every sample. Now:
--   - The intake marks points of series it already forwarded in their hour (resource attribute
--     oc.series_known = 1, from its x-oc-series-known header); the series views skip them. A series
--     gets about one row per hour, and its LastSeen is a time inside every hour it reported, so
--     readers test "seen since t" as LastSeen >= toStartOfHour(t). Samples are unchanged.
--   - Rollups are plain-value ReplacingMergeTree tables filled by the server's rollup job from the
--     samples (5 minutes) and from the 5-minute rollups (1 hour), once a bucket is closed; re-running
--     a bucket replaces its rows, so readers read them with FINAL.
--   - Samples carry all three retention tiers, so the job writes rollups with the org's tiers.
--   - The exporter's own metric tables (otel.otel_metrics_*), unused since ADR-0001, are dropped.
-- Existing rollups are dropped with their views (metrics start fresh, as ADR-0002 decided).

-- ================================================================ views replaced below

DROP VIEW IF EXISTS otel.metrics_points_gauge_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_points_sum_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_series_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_samples_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_rollup_5m_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_rollup_1h_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_points_histogram_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_points_exponential_histogram_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_points_summary_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_series_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_samples_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_rollup_5m_mv ${ON_CLUSTER} SYNC;
DROP VIEW IF EXISTS otel.metrics_dist_rollup_1h_mv ${ON_CLUSTER} SYNC;

-- The normalized point tables (Null: nothing stored) gain the mark.
DROP TABLE IF EXISTS otel.metrics_points${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_dist_points${LOCAL} ${ON_CLUSTER} SYNC;

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
    `Retention1hDays` UInt16,
    -- The intake already forwarded this series in the point's hour: no series row needed.
    `SeriesKnown` Bool
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
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays,
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown
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
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays,
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown
FROM otel.metrics_ingest_sum${LOCAL} AS src;

-- ================================================================ series

-- One row per series and insert of unmarked points; merges keep the latest.
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
WHERE NOT SeriesKnown
GROUP BY OrgId, MetricName, SeriesId;

-- ================================================================ samples

ALTER TABLE otel.metrics_samples${LOCAL} ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS `Retention5mDays` UInt16, ADD COLUMN IF NOT EXISTS `Retention1hDays` UInt16;
-- The Distributed table in a cluster (on a single server the same table: a no-op).
ALTER TABLE otel.metrics_samples ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS `Retention5mDays` UInt16, ADD COLUMN IF NOT EXISTS `Retention1hDays` UInt16;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_samples_mv ${ON_CLUSTER} TO otel.metrics_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Value, RetentionDays, Retention5mDays, Retention1hDays
FROM otel.metrics_points${LOCAL};

-- ================================================================ rollups (written by the rollup job)

DROP TABLE IF EXISTS otel.metrics_rollup_5m ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_rollup_5m${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_rollup_1h ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_rollup_1h${LOCAL} ${ON_CLUSTER} SYNC;

-- Per series and 5 minutes: min / max / sum / count, the first and last value and their times
-- (Start, End: rates divide by the time the samples span, as on raw samples).
CREATE TABLE IF NOT EXISTS otel.metrics_rollup_5m${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `Min` Float64 CODEC(Gorilla, ZSTD(1)),
    `Max` Float64 CODEC(Gorilla, ZSTD(1)),
    `Sum` Float64 CODEC(Gorilla, ZSTD(1)),
    `Count` UInt64 CODEC(T64, ZSTD(1)),
    `First` Float64 CODEC(Gorilla, ZSTD(1)),
    `Last` Float64 CODEC(Gorilla, ZSTD(1)),
    `Start` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `End` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16,
    -- The org's 1-hour tier, which the hour rolled up from these rows keeps.
    `Retention1hDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toDate(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 90, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_rollup_5m SeriesId};

-- Monthly partitions: a year is 12 partitions per retention, not 365.
CREATE TABLE IF NOT EXISTS otel.metrics_rollup_1h${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `Min` Float64 CODEC(Gorilla, ZSTD(1)),
    `Max` Float64 CODEC(Gorilla, ZSTD(1)),
    `Sum` Float64 CODEC(Gorilla, ZSTD(1)),
    `Count` UInt64 CODEC(T64, ZSTD(1)),
    `First` Float64 CODEC(Gorilla, ZSTD(1)),
    `Last` Float64 CODEC(Gorilla, ZSTD(1)),
    `Start` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `End` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toYYYYMM(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_rollup_1h SeriesId};

-- ================================================================ distributions

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
    `Retention1hDays` UInt16,
    `SeriesKnown` Bool
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
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays,
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown
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
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays,
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown
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
    toUInt16OrZero(src.ResourceAttributes['oc.metrics_retention_1h_days']) AS Retention1hDays,
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown
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
WHERE NOT SeriesKnown
GROUP BY OrgId, MetricName, SeriesId;

ALTER TABLE otel.metrics_dist_samples${LOCAL} ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS `Retention5mDays` UInt16, ADD COLUMN IF NOT EXISTS `Retention1hDays` UInt16;
ALTER TABLE otel.metrics_dist_samples ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS `Retention5mDays` UInt16, ADD COLUMN IF NOT EXISTS `Retention1hDays` UInt16;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_samples_mv ${ON_CLUSTER} TO otel.metrics_dist_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Count, Sum, Min, Max, BucketCounts, ExplicitBounds, Scale, ZeroCount, PositiveOffset, PositiveBucketCounts,
       NegativeOffset, NegativeBucketCounts, Quantiles AS `ValueAtQuantiles.Quantile`, QuantileValues AS `ValueAtQuantiles.Value`, RetentionDays, Retention5mDays, Retention1hDays
FROM otel.metrics_dist_points${LOCAL};

DROP TABLE IF EXISTS otel.metrics_dist_rollup_5m ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_dist_rollup_5m${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_dist_rollup_1h ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.metrics_dist_rollup_1h${LOCAL} ${ON_CLUSTER} SYNC;

-- Per series and bucket: the first and last cumulative value (a cumulative series' change is
-- last − first), the total (a delta series' sum), min / max, the bucket layout and the summaries'
-- quantile totals, so every statistic of the raw points can be computed from it.
CREATE TABLE IF NOT EXISTS otel.metrics_dist_rollup_5m${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `CountFirst` UInt64 CODEC(ZSTD(1)),
    `CountLast` UInt64 CODEC(ZSTD(1)),
    `CountTotal` UInt64 CODEC(ZSTD(1)),
    `SumFirst` Float64 CODEC(ZSTD(1)),
    `SumLast` Float64 CODEC(ZSTD(1)),
    `SumTotal` Float64 CODEC(ZSTD(1)),
    `Min` Float64 CODEC(ZSTD(1)),
    `Max` Float64 CODEC(ZSTD(1)),
    -- Histogram buckets, or an exponential histogram's positive buckets.
    `BucketsFirst` Array(UInt64) CODEC(ZSTD(1)),
    `BucketsLast` Array(UInt64) CODEC(ZSTD(1)),
    `BucketsTotal` Array(UInt64) CODEC(ZSTD(1)),
    `ExplicitBounds` Array(Float64) CODEC(ZSTD(1)),
    `Scale` Int32,
    `PositiveOffset` Int32,
    -- Summaries: the reported quantiles and the sum of their values over the bucket's points.
    `Quantiles` Array(Float64) CODEC(ZSTD(1)),
    `QuantileTotals` Array(Float64) CODEC(ZSTD(1)),
    `Points` UInt64,
    `Start` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `End` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16,
    `Retention1hDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toDate(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 90, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_dist_rollup_5m SeriesId};

CREATE TABLE IF NOT EXISTS otel.metrics_dist_rollup_1h${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `Bucket` DateTime CODEC(Delta, ZSTD(1)),
    `CountFirst` UInt64 CODEC(ZSTD(1)),
    `CountLast` UInt64 CODEC(ZSTD(1)),
    `CountTotal` UInt64 CODEC(ZSTD(1)),
    `SumFirst` Float64 CODEC(ZSTD(1)),
    `SumLast` Float64 CODEC(ZSTD(1)),
    `SumTotal` Float64 CODEC(ZSTD(1)),
    `Min` Float64 CODEC(ZSTD(1)),
    `Max` Float64 CODEC(ZSTD(1)),
    `BucketsFirst` Array(UInt64) CODEC(ZSTD(1)),
    `BucketsLast` Array(UInt64) CODEC(ZSTD(1)),
    `BucketsTotal` Array(UInt64) CODEC(ZSTD(1)),
    `ExplicitBounds` Array(Float64) CODEC(ZSTD(1)),
    `Scale` Int32,
    `PositiveOffset` Int32,
    `Quantiles` Array(Float64) CODEC(ZSTD(1)),
    `QuantileTotals` Array(Float64) CODEC(ZSTD(1)),
    `Points` UInt64,
    `Start` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `End` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `RetentionDays` UInt16
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toYYYYMM(Bucket), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, Bucket)
TTL Bucket + toIntervalDay(if(RetentionDays = 0, 395, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_dist_rollup_1h SeriesId};

-- ================================================================ the exporter's own tables

DROP TABLE IF EXISTS otel.otel_metrics_gauge ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_gauge${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_sum ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_sum${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_histogram ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_histogram${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_exponential_histogram ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_exponential_histogram${LOCAL} ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_summary ${ON_CLUSTER} SYNC;
DROP TABLE IF EXISTS otel.otel_metrics_summary${LOCAL} ${ON_CLUSTER} SYNC
