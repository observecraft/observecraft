-- Kubernetes inventory for the Kubernetes Explorer.
-- The cluster collector's k8sobjects receiver sends every object (pods, deployments, nodes, …) each
-- minute and events as they happen; the ingest Collector routes them (resource oc.signal = k8s.objects)
-- to otel_k8s_objects (exporter schema v0.112, kept one day). Materialized views keep:
--   k8s_resources  the latest state of each object (ReplacingMergeTree on LastSeen)
--   k8s_events     Kubernetes events
CREATE TABLE IF NOT EXISTS otel.otel_k8s_objects
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
ENGINE = MergeTree
PARTITION BY toDate(TimestampTime)
ORDER BY (OrgId, TimestampTime)
TTL TimestampTime + INTERVAL 1 DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;

CREATE TABLE IF NOT EXISTS otel.k8s_resources
(
    `OrgId` LowCardinality(String),
    `Cluster` LowCardinality(String),
    `Kind` LowCardinality(String),
    `Namespace` String,
    `Name` String,
    `Uid` String,
    `Body` String CODEC(ZSTD(3)),
    `LastSeen` DateTime64(3)
)
ENGINE = ReplacingMergeTree(LastSeen)
ORDER BY (OrgId, Cluster, Kind, Namespace, Name, Uid)
TTL toDateTime(LastSeen) + INTERVAL 7 DAY;

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.k8s_resources_mv TO otel.k8s_resources AS
SELECT
    ResourceAttributes['oc.org_id'] AS OrgId,
    ResourceAttributes['k8s.cluster.name'] AS Cluster,
    JSONExtractString(Body, 'kind') AS Kind,
    JSONExtractString(Body, 'metadata', 'namespace') AS Namespace,
    JSONExtractString(Body, 'metadata', 'name') AS Name,
    JSONExtractString(Body, 'metadata', 'uid') AS Uid,
    Body,
    now64(3) AS LastSeen
FROM otel.otel_k8s_objects
WHERE JSONExtractString(Body, 'kind') NOT IN ('', 'Event') AND JSONExtractString(Body, 'metadata', 'uid') != '';

CREATE TABLE IF NOT EXISTS otel.k8s_events
(
    `OrgId` LowCardinality(String),
    `Cluster` LowCardinality(String),
    `Timestamp` DateTime64(3),
    `Type` LowCardinality(String),
    `Reason` LowCardinality(String),
    `Note` String,
    `RegardingKind` LowCardinality(String),
    `RegardingNamespace` String,
    `RegardingName` String,
    `RegardingUid` String,
    `Count` UInt32,
    `Body` String CODEC(ZSTD(3))
)
ENGINE = MergeTree
PARTITION BY toDate(Timestamp)
ORDER BY (OrgId, Cluster, RegardingUid, Timestamp)
TTL toDateTime(Timestamp) + INTERVAL 7 DAY;

-- Watch-mode records are {"type": "ADDED" | "MODIFIED" | …, "object": <events.k8s.io/v1 Event>}.
-- (The inner query keeps the source Body from being shadowed by the output column of the same name.)
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.k8s_events_mv TO otel.k8s_events AS
SELECT
    OrgId, Cluster,
    coalesce(
        parseDateTime64BestEffortOrNull(JSONExtractString(Event, 'deprecatedLastTimestamp'), 3),
        parseDateTime64BestEffortOrNull(JSONExtractString(Event, 'eventTime'), 3),
        now64(3)) AS Timestamp,
    JSONExtractString(Event, 'type') AS Type,
    JSONExtractString(Event, 'reason') AS Reason,
    JSONExtractString(Event, 'note') AS Note,
    JSONExtractString(Event, 'regarding', 'kind') AS RegardingKind,
    JSONExtractString(Event, 'regarding', 'namespace') AS RegardingNamespace,
    JSONExtractString(Event, 'regarding', 'name') AS RegardingName,
    JSONExtractString(Event, 'regarding', 'uid') AS RegardingUid,
    toUInt32(greatest(JSONExtractInt(Event, 'deprecatedCount'), 1)) AS Count,
    Event AS Body
FROM
(
    SELECT ResourceAttributes['oc.org_id'] AS OrgId, ResourceAttributes['k8s.cluster.name'] AS Cluster, JSONExtractRaw(Body, 'object') AS Event
    FROM otel.otel_k8s_objects
    WHERE JSONExtractString(Body, 'object', 'kind') = 'Event'
);
