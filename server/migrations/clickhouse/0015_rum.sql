-- Real User Monitoring events (views, actions, errors, resources, long tasks, vitals) from the browser
-- and mobile SDKs (`POST /api/v2/rum` on the intake), sent as OTLP log records with resource
-- oc.signal = dd.rum and routed here by the collector. Same columns as the collector's logs table:
-- Body is the event as the SDK sent it (JSON), LogAttributes its fields flattened to dotted paths
-- (`view.name`, `session.id`, `error.message`, …) plus what the intake derived (browser, OS and device
-- from the user agent, `network.client.ip`). A view is sent again each time it changes; readers keep
-- the latest `_dd.document_version` of each `view.id`. Kept 30 days, like Datadog's RUM events.
CREATE TABLE IF NOT EXISTS otel.otel_rum${LOCAL} ${ON_CLUSTER}
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
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    `ApplicationId` LowCardinality(String) MATERIALIZED LogAttributes['application.id'],
    `EventType` LowCardinality(String) MATERIALIZED LogAttributes['type'],
    `SessionId` String MATERIALIZED LogAttributes['session.id'] CODEC(ZSTD(1)),
    `ViewId` String MATERIALIZED LogAttributes['view.id'] CODEC(ZSTD(1)),
    INDEX idx_session SessionId TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_view ViewId TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimestampTime)
ORDER BY (OrgId, ApplicationId, EventType, TimestampTime)
TTL TimestampTime + INTERVAL 30 DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_rum rand()};
