-- Data Streams Monitoring pathway checkpoints: the stats points the tracers aggregate every 10 seconds
-- (`/api/v0.1/pipeline_stats`, via the Agent), sent by the intake as OTLP log records with resource
-- oc.signal = dd.pathways and routed here by the collector. Same columns as the collector's logs table;
-- LogAttributes hold a point's edge tags, pathway hash and parent hash, checkpoint count and its sketches' bins
-- (edge and pathway latency, payload size), or a Kafka backlog's tags and offset. Kept 15 days.
CREATE TABLE IF NOT EXISTS otel.otel_pathways${LOCAL} ${ON_CLUSTER}
(
    `Timestamp` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimestampTime` DateTime DEFAULT toDateTime(Timestamp),
    `TraceId` String CODEC(ZSTD(1)),
    `SpanId` String CODEC(ZSTD(1)),
    `TraceFlags` UInt8,
    `SeverityText` LowCardinality(String) CODEC(ZSTD(1)),
    `SeverityNumber` UInt8,
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `Body` String CODEC(ZSTD(1)),
    `ResourceSchemaUrl` LowCardinality(String) CODEC(ZSTD(1)),
    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeSchemaUrl` LowCardinality(String) CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` LowCardinality(String) CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `LogAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id']
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimestampTime)
ORDER BY (OrgId, TimestampTime)
TTL TimestampTime + INTERVAL 15 DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_pathways rand()};
