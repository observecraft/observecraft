-- Session Replay segments of the browser SDK (`POST /api/v2/replay` on the intake): each segment is a
-- run of DOM snapshots and incremental changes (mutations, mouse moves, clicks, scrolls, inputs) of one
-- view, sent as an OTLP log record with resource oc.signal = dd.replay and routed here by the
-- collector. Body is the segment's JSON (its `records`); LogAttributes carry the application, session
-- and view, the segment's start and end and whether it starts with a full snapshot. Read per session
-- (the player) and per view (heatmaps). Kept 30 days, like the RUM events.
CREATE TABLE IF NOT EXISTS otel.otel_rum_replay${LOCAL} ${ON_CLUSTER}
(
    `Timestamp` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimestampTime` DateTime DEFAULT toDateTime(Timestamp),
    `TraceId` String CODEC(ZSTD(1)),
    `SpanId` String CODEC(ZSTD(1)),
    `TraceFlags` UInt8,
    `SeverityText` LowCardinality(String) CODEC(ZSTD(1)),
    `SeverityNumber` UInt8,
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `Body` String CODEC(ZSTD(3)),
    `ResourceSchemaUrl` LowCardinality(String) CODEC(ZSTD(1)),
    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeSchemaUrl` LowCardinality(String) CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` LowCardinality(String) CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `LogAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    `ApplicationId` LowCardinality(String) MATERIALIZED LogAttributes['application.id'],
    `SessionId` String MATERIALIZED LogAttributes['session.id'] CODEC(ZSTD(1)),
    `ViewId` String MATERIALIZED LogAttributes['view.id'] CODEC(ZSTD(1)),
    INDEX idx_view ViewId TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimestampTime)
ORDER BY (OrgId, SessionId, Timestamp)
TTL TimestampTime + INTERVAL 30 DAY
SETTINGS index_granularity = 1024, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_rum_replay cityHash64(SessionId)};
