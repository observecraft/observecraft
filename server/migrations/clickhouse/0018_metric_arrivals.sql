-- Track arrival time independently of event time. Kafka replay can insert samples into buckets
-- the rollup job has already closed. One marker per org, arrival minute and five-minute bucket
-- survives merges; this is independent of series cardinality and does not retain sample values.
CREATE TABLE IF NOT EXISTS otel.metrics_arrivals${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `ArrivalMinute` DateTime('UTC'),
    `Bucket` DateTime('UTC')
)
ENGINE = ${REPLICATED}ReplacingMergeTree
PARTITION BY toDate(ArrivalMinute)
ORDER BY (OrgId, ArrivalMinute, Bucket)
TTL ArrivalMinute + INTERVAL 30 DAY
SETTINGS ttl_only_drop_parts = 1;
${DISTRIBUTED otel.metrics_arrivals cityHash64(OrgId, Bucket)};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_arrivals_scalar_mv${LOCAL} ${ON_CLUSTER}
TO otel.metrics_arrivals${LOCAL} AS
SELECT OrgId, toStartOfMinute(now('UTC')) AS ArrivalMinute,
       toStartOfFiveMinutes(TimeUnix) AS Bucket
FROM otel.metrics_samples${LOCAL}
GROUP BY OrgId, ArrivalMinute, Bucket;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.metrics_arrivals_dist_mv${LOCAL} ${ON_CLUSTER}
TO otel.metrics_arrivals${LOCAL} AS
SELECT OrgId, toStartOfMinute(now('UTC')) AS ArrivalMinute,
       toStartOfFiveMinutes(TimeUnix) AS Bucket
FROM otel.metrics_dist_samples${LOCAL}
GROUP BY OrgId, ArrivalMinute, Bucket;
