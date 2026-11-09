-- Screenshots (one per step, JPEG) and videos (WebM) of browser test runs, as bytes, written by the server after
-- it records a run (Postgres synthetic_results.id). Kept with failed runs and runs started by hand, 30 days like
-- the results.
CREATE TABLE IF NOT EXISTS otel.synthetics_artifacts${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Timestamp` DateTime64(3, 'UTC') CODEC(Delta(8), ZSTD(1)),
    `TestId` String CODEC(ZSTD(1)),
    `ResultId` Int64,
    `Name` String CODEC(ZSTD(1)),
    `ContentType` LowCardinality(String),
    `Data` String CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Timestamp)
ORDER BY (OrgId, TestId, ResultId, Name)
TTL toDateTime(Timestamp) + INTERVAL 30 DAY
SETTINGS ttl_only_drop_parts = 1;
${DISTRIBUTED otel.synthetics_artifacts cityHash64(TestId)};
