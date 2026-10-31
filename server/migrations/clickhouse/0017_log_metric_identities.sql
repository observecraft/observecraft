-- Replay-safe generated metrics retain one canonical contribution per stable source event.
-- Normalized OTel points still feed the existing series/tag catalog. Only event-bearing samples
-- take the replacing path; ordinary metrics and older aggregate outbox payloads keep their path.
ALTER TABLE otel.metrics_points${LOCAL} ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS EventId String DEFAULT '';
ALTER TABLE otel.metrics_dist_points${LOCAL} ${ON_CLUSTER} ADD COLUMN IF NOT EXISTS EventId String DEFAULT '';

CREATE TABLE IF NOT EXISTS otel.metrics_log_samples${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `MetricName` LowCardinality(String),
    `SeriesId` UInt64,
    `TimeUnix` DateTime64(3) CODEC(Delta, ZSTD(1)),
    `Value` Float64 CODEC(Gorilla, ZSTD(1)),
    `RetentionDays` UInt16,
    `Retention5mDays` UInt16,
    `Retention1hDays` UInt16,
    `EventId` String CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toDate(TimeUnix), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, TimeUnix, EventId)
TTL toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, 30, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_log_samples SeriesId};

CREATE TABLE IF NOT EXISTS otel.metrics_log_dist_samples${LOCAL} ${ON_CLUSTER}
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
    `RetentionDays` UInt16,
    `Retention5mDays` UInt16,
    `Retention1hDays` UInt16,
    `EventId` String CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY (toDate(TimeUnix), RetentionDays)
ORDER BY (OrgId, MetricName, SeriesId, TimeUnix, EventId)
TTL toDateTime(TimeUnix) + toIntervalDay(if(RetentionDays = 0, 30, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_log_dist_samples SeriesId};

DROP VIEW IF EXISTS otel.metrics_samples_mv ${ON_CLUSTER} SYNC;
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_samples_mv ${ON_CLUSTER} TO otel.metrics_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Value, RetentionDays, Retention5mDays, Retention1hDays
FROM otel.metrics_points${LOCAL}
WHERE EventId = '';

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_log_samples_mv ${ON_CLUSTER} TO otel.metrics_log_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Value, RetentionDays, Retention5mDays, Retention1hDays, EventId
FROM otel.metrics_points${LOCAL}
WHERE EventId != '';

DROP VIEW IF EXISTS otel.metrics_dist_samples_mv ${ON_CLUSTER} SYNC;
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_samples_mv ${ON_CLUSTER} TO otel.metrics_dist_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Count, Sum, Min, Max, BucketCounts, ExplicitBounds, Scale, ZeroCount, PositiveOffset, PositiveBucketCounts,
       NegativeOffset, NegativeBucketCounts, Quantiles AS `ValueAtQuantiles.Quantile`, QuantileValues AS `ValueAtQuantiles.Value`, RetentionDays, Retention5mDays, Retention1hDays
FROM otel.metrics_dist_points${LOCAL}
WHERE EventId = '';

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_log_samples_mv ${ON_CLUSTER} TO otel.metrics_log_dist_samples${LOCAL} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, Count, Sum, Min, Max, BucketCounts, ExplicitBounds, Scale, ZeroCount, PositiveOffset, PositiveBucketCounts,
       NegativeOffset, NegativeBucketCounts, Quantiles AS `ValueAtQuantiles.Quantile`, QuantileValues AS `ValueAtQuantiles.Value`, RetentionDays, Retention5mDays, Retention1hDays, EventId
FROM otel.metrics_dist_points${LOCAL}
WHERE EventId != '';

CREATE TABLE IF NOT EXISTS otel.metrics_log_ingest_sum${LOCAL} ${ON_CLUSTER} AS otel.metrics_ingest_sum${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_log_ingest_sum cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_log_points_sum_mv ${ON_CLUSTER} TO otel.metrics_points${LOCAL} AS
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
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown,
    src.ResourceAttributes['oc.log_event_id'] AS EventId
FROM otel.metrics_log_ingest_sum${LOCAL} AS src;

CREATE TABLE IF NOT EXISTS otel.metrics_log_ingest_exponential_histogram${LOCAL} ${ON_CLUSTER} AS otel.metrics_ingest_exponential_histogram${LOCAL} ENGINE = Null;
${DISTRIBUTED otel.metrics_log_ingest_exponential_histogram cityHash64(ResourceAttributes['oc.org_id'], MetricName, mapSort(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes)), mapSort(Attributes))};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_dist_log_points_exponential_histogram_mv ${ON_CLUSTER} TO otel.metrics_dist_points${LOCAL} AS
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
    src.ResourceAttributes['oc.series_known'] = '1' AS SeriesKnown,
    src.ResourceAttributes['oc.log_event_id'] AS EventId
FROM otel.metrics_log_ingest_exponential_histogram${LOCAL} AS src;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_log_arrivals_scalar_mv${LOCAL} ${ON_CLUSTER} TO otel.metrics_arrivals${LOCAL} AS
SELECT OrgId, toStartOfMinute(now('UTC')) AS ArrivalMinute, toStartOfFiveMinutes(TimeUnix) AS Bucket
FROM otel.metrics_log_samples${LOCAL}
GROUP BY OrgId, ArrivalMinute, Bucket;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_log_arrivals_dist_mv${LOCAL} ${ON_CLUSTER} TO otel.metrics_arrivals${LOCAL} AS
SELECT OrgId, toStartOfMinute(now('UTC')) AS ArrivalMinute, toStartOfFiveMinutes(TimeUnix) AS Bucket
FROM otel.metrics_log_dist_samples${LOCAL}
GROUP BY OrgId, ArrivalMinute, Bucket;

-- Deduplicate on each owning shard before aggregating contributions into 10-second points.
CREATE VIEW IF NOT EXISTS otel.metrics_log_samples_read${LOCAL} ${ON_CLUSTER} AS
SELECT OrgId, MetricName, SeriesId, TimeUnix, sum(Value) AS Value, RetentionDays, Retention5mDays, Retention1hDays
FROM otel.metrics_log_samples${LOCAL} FINAL
GROUP BY OrgId, MetricName, SeriesId, TimeUnix, RetentionDays, Retention5mDays, Retention1hDays;
${DISTRIBUTED otel.metrics_log_samples_read SeriesId};

CREATE VIEW IF NOT EXISTS otel.metrics_log_dist_samples_read${LOCAL} ${ON_CLUSTER} AS
WITH if(empty(positive), toInt32(0), arrayMin(mapKeys(positive))) AS pos_lo,
     if(empty(negative), toInt32(0), arrayMin(mapKeys(negative))) AS neg_lo
SELECT OrgId, MetricName, SeriesId, TimeUnix, Count, Sum, Min, Max,
       CAST([], 'Array(UInt64)') AS BucketCounts, CAST([], 'Array(Float64)') AS ExplicitBounds,
       Scale, ZeroCount, pos_lo AS PositiveOffset,
       arrayMap(i -> positive[toInt32(i)], range(toInt64(pos_lo), if(empty(positive), toInt64(pos_lo), toInt64(arrayMax(mapKeys(positive))) + 1))) AS PositiveBucketCounts,
       neg_lo AS NegativeOffset,
       arrayMap(i -> negative[toInt32(i)], range(toInt64(neg_lo), if(empty(negative), toInt64(neg_lo), toInt64(arrayMax(mapKeys(negative))) + 1))) AS NegativeBucketCounts,
       CAST([], 'Array(Float64)') AS `ValueAtQuantiles.Quantile`, CAST([], 'Array(Float64)') AS `ValueAtQuantiles.Value`,
       RetentionDays, Retention5mDays, Retention1hDays
FROM (
    SELECT OrgId, MetricName, SeriesId, TimeUnix, sum(Count) AS Count, sum(Sum) AS Sum, min(Min) AS Min, max(Max) AS Max,
           any(Scale) AS Scale, sum(ZeroCount) AS ZeroCount,
           sumMap(mapFromArrays(arrayMap(i -> toInt32(PositiveOffset + toInt64(i)), range(length(PositiveBucketCounts))), PositiveBucketCounts)) AS positive,
           sumMap(mapFromArrays(arrayMap(i -> toInt32(NegativeOffset + toInt64(i)), range(length(NegativeBucketCounts))), NegativeBucketCounts)) AS negative,
           RetentionDays, Retention5mDays, Retention1hDays
    FROM otel.metrics_log_dist_samples${LOCAL} FINAL
    GROUP BY OrgId, MetricName, SeriesId, TimeUnix, RetentionDays, Retention5mDays, Retention1hDays
);
${DISTRIBUTED otel.metrics_log_dist_samples_read SeriesId};

-- Keep the common reader Distributed over a local union. Distributed joins and the rollup
-- worker's parallel INSERT SELECT then stay on each series' owning shard.
CREATE VIEW IF NOT EXISTS otel.metrics_samples_read${LOCAL} ${ON_CLUSTER} AS
SELECT * FROM otel.metrics_samples${LOCAL} UNION ALL SELECT * FROM otel.metrics_log_samples_read${LOCAL};
${DISTRIBUTED otel.metrics_samples_read SeriesId};
CREATE VIEW IF NOT EXISTS otel.metrics_dist_samples_read${LOCAL} ${ON_CLUSTER} AS
SELECT * FROM otel.metrics_dist_samples${LOCAL} UNION ALL SELECT * FROM otel.metrics_log_dist_samples_read${LOCAL};
${DISTRIBUTED otel.metrics_dist_samples_read SeriesId};
