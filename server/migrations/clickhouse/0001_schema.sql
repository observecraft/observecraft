-- ObserveCraft telemetry schema (ClickHouse). Applied by oc-store, which renders the placeholders:
--   ${ON_CLUSTER}   empty, or ON CLUSTER '<cluster>' when OC_CLICKHOUSE_CLUSTER is set
--   ${LOCAL}        empty, or "_local": in a cluster each shard stores data in <table>_local
--   ${REPLICATED}   empty, or "Replicated" (ReplicatedMergeTree & co. with the server's default
--                   replica path and the {replica} macro)
--   ${DISTRIBUTED db.table sharding_key}
--                   nothing on a single node; in a cluster, the Distributed table <db.table> over
--                   <db.table>_local that every reader and writer uses
-- so readers and writers always use the plain names below.
--
-- Multi-tenancy: every table leads its sort key with the organization, so a query of one org reads
-- only that org's granules; partitions are by day (and by retention where rows carry their own),
-- so expired data is dropped as whole parts without rewriting anything.
--
-- Sharding (cluster): spans by TraceId (a trace stays on one shard), metrics by series (rates and
-- deltas need a series' points together), Kubernetes objects by uid (latest-state dedup), logs
-- evenly. Every shard sorts its data by org, so per-org reads stay cheap on all of them.

CREATE DATABASE IF NOT EXISTS oc ${ON_CLUSTER};
CREATE DATABASE IF NOT EXISTS otel ${ON_CLUSTER};

-- ================================================================ logs

-- Indexed logs (written by the log indexer). Each row carries its index's retention; partitions
-- split by retention so a 3-day index's parts don't wait for a 30-day index's.
CREATE TABLE IF NOT EXISTS oc.logs${LOCAL} ${ON_CLUSTER}
(
    org_id          LowCardinality(String),
    id              UUID DEFAULT generateUUIDv4(),
    timestamp       DateTime64(3, 'UTC') CODEC(Delta(8), ZSTD(1)),
    status          LowCardinality(String),
    service         LowCardinality(String),
    host            LowCardinality(String),
    source          LowCardinality(String),
    message         String CODEC(ZSTD(1)),
    -- Infrastructure tags (env, kube_namespace, pod_name, …), queried as key:value.
    tags            Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    -- Flattened event attributes (http.status_code, …), queried as @path:value.
    attr            Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    -- Original attributes as JSON, for display.
    attributes      String CODEC(ZSTD(3)),
    trace_id        String DEFAULT '' CODEC(ZSTD(1)),
    span_id         String DEFAULT '' CODEC(ZSTD(1)),
    severity_number UInt8 DEFAULT 0,
    scope           LowCardinality(String) DEFAULT '',
    index_name      LowCardinality(String) DEFAULT 'main',
    retention_days  UInt16 DEFAULT 15,

    INDEX idx_message lower(message) TYPE ngrambf_v1(3, 65536, 3, 0) GRANULARITY 1,
    INDEX idx_tag_keys mapKeys(tags) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_tag_values mapValues(tags) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_keys mapKeys(attr) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_values mapValues(attr) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_host host TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.001) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY (toDate(timestamp), retention_days)
ORDER BY (org_id, service, timestamp)
TTL toDateTime(timestamp) + toIntervalDay(retention_days)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED oc.logs rand()};

-- Every ingested log for a few days, indexed or not (Vector writes here; archives read it).
CREATE TABLE IF NOT EXISTS oc.logs_raw${LOCAL} ${ON_CLUSTER} AS oc.logs${LOCAL}
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(timestamp)
ORDER BY (org_id, service, timestamp)
TTL toDateTime(timestamp) + INTERVAL 72 HOUR
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED oc.logs_raw rand()};

-- Logs read back from archives. One partition per rehydration: expiring one drops its partition.
CREATE TABLE IF NOT EXISTS oc.logs_rehydrated${LOCAL} ${ON_CLUSTER}
(
    org_id          LowCardinality(String),
    id              UUID DEFAULT generateUUIDv4(),
    timestamp       DateTime64(3, 'UTC') CODEC(Delta(8), ZSTD(1)),
    status          LowCardinality(String),
    service         LowCardinality(String),
    host            LowCardinality(String),
    source          LowCardinality(String),
    message         String CODEC(ZSTD(1)),
    tags            Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    attr            Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    attributes      String CODEC(ZSTD(3)),
    trace_id        String DEFAULT '' CODEC(ZSTD(1)),
    span_id         String DEFAULT '' CODEC(ZSTD(1)),
    severity_number UInt8 DEFAULT 0,
    scope           LowCardinality(String) DEFAULT '',
    index_name      LowCardinality(String) DEFAULT 'rehydrated',
    retention_days  UInt16 DEFAULT 0,
    rehydration_id  String DEFAULT '',

    INDEX idx_message lower(message) TYPE ngrambf_v1(3, 65536, 3, 0) GRANULARITY 1,
    INDEX idx_tag_keys mapKeys(tags) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_tag_values mapValues(tags) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_keys mapKeys(attr) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_values mapValues(attr) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.001) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY rehydration_id
ORDER BY (org_id, service, timestamp)
SETTINGS index_granularity = 8192;
${DISTRIBUTED oc.logs_rehydrated rand()};

-- ================================================================ spans
-- Columns are the OTel Collector ClickHouse exporter's (v0.112, create_schema: false); OrgId and
-- RetentionDays come from the resource attributes the ingest gateway stamps.

CREATE TABLE IF NOT EXISTS otel.otel_traces${LOCAL} ${ON_CLUSTER}
(

    `Timestamp` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TraceId` String CODEC(ZSTD(1)),
    `SpanId` String CODEC(ZSTD(1)),
    `ParentSpanId` String CODEC(ZSTD(1)),
    `TraceState` String CODEC(ZSTD(1)),
    `SpanName` LowCardinality(String) CODEC(ZSTD(1)),
    `SpanKind` LowCardinality(String) CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `SpanAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `Duration` UInt64 CODEC(ZSTD(1)),
    `StatusCode` LowCardinality(String) CODEC(ZSTD(1)),
    `StatusMessage` String CODEC(ZSTD(1)),
    `Events.Timestamp` Array(DateTime64(9)) CODEC(ZSTD(1)),
    `Events.Name` Array(LowCardinality(String)) CODEC(ZSTD(1)),
    `Events.Attributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `Links.TraceId` Array(String) CODEC(ZSTD(1)),
    `Links.SpanId` Array(String) CODEC(ZSTD(1)),
    `Links.TraceState` Array(String) CODEC(ZSTD(1)),
    `Links.Attributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    -- The org's span retention (0 = the install default).
    `RetentionDays` UInt16 MATERIALIZED toUInt16OrZero(ResourceAttributes['oc.retention_days']),
    INDEX idx_trace_id TraceId TYPE bloom_filter(0.001) GRANULARITY 1,
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_span_attr_key mapKeys(SpanAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_span_attr_value mapValues(SpanAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_duration Duration TYPE minmax GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY (toDate(Timestamp), RetentionDays)
ORDER BY (OrgId, ServiceName, SpanName, toDateTime(Timestamp))
TTL toDate(Timestamp) + toIntervalDay(if(RetentionDays = 0, 15, RetentionDays))
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_traces cityHash64(TraceId)};

-- CI pipeline spans (routed by the ingest Collector).
CREATE TABLE IF NOT EXISTS otel.otel_ci_traces${LOCAL} ${ON_CLUSTER}
(

    `Timestamp` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TraceId` String CODEC(ZSTD(1)),
    `SpanId` String CODEC(ZSTD(1)),
    `ParentSpanId` String CODEC(ZSTD(1)),
    `TraceState` String CODEC(ZSTD(1)),
    `SpanName` LowCardinality(String) CODEC(ZSTD(1)),
    `SpanKind` LowCardinality(String) CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `SpanAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `Duration` UInt64 CODEC(ZSTD(1)),
    `StatusCode` LowCardinality(String) CODEC(ZSTD(1)),
    `StatusMessage` String CODEC(ZSTD(1)),
    `Events.Timestamp` Array(DateTime64(9)) CODEC(ZSTD(1)),
    `Events.Name` Array(LowCardinality(String)) CODEC(ZSTD(1)),
    `Events.Attributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `Links.TraceId` Array(String) CODEC(ZSTD(1)),
    `Links.SpanId` Array(String) CODEC(ZSTD(1)),
    `Links.TraceState` Array(String) CODEC(ZSTD(1)),
    `Links.Attributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    INDEX idx_trace_id TraceId TYPE bloom_filter(0.001) GRANULARITY 1,
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_span_attr_key mapKeys(SpanAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_span_attr_value mapValues(SpanAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_duration Duration TYPE minmax GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Timestamp)
ORDER BY (OrgId, ServiceName, SpanName, toDateTime(Timestamp))
TTL toDate(Timestamp) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_ci_traces cityHash64(TraceId)};

-- Trace id → time bounds, so a trace lookup prunes partitions. Kept per org.
CREATE TABLE IF NOT EXISTS otel.otel_traces_trace_id_ts${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `TraceId` String CODEC(ZSTD(1)),
    `Start` DateTime CODEC(Delta(4), ZSTD(1)),
    `End` DateTime CODEC(Delta(4), ZSTD(1)),
    INDEX idx_trace_id TraceId TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Start)
ORDER BY (OrgId, TraceId, Start)
TTL toDate(Start) + toIntervalDay(90)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_traces_trace_id_ts cityHash64(TraceId)};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.otel_traces_trace_id_ts_mv ${ON_CLUSTER} TO otel.otel_traces_trace_id_ts${LOCAL}
(
    `OrgId` LowCardinality(String),
    `TraceId` String,
    `Start` DateTime64(9),
    `End` DateTime64(9)
)
AS SELECT
    ResourceAttributes['oc.org_id'] AS OrgId,
    TraceId,
    min(Timestamp) AS Start,
    max(Timestamp) AS End
FROM otel.otel_traces${LOCAL}
WHERE TraceId != ''
GROUP BY OrgId, TraceId;

CREATE TABLE IF NOT EXISTS otel.otel_ci_traces_trace_id_ts${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `TraceId` String CODEC(ZSTD(1)),
    `Start` DateTime CODEC(Delta(4), ZSTD(1)),
    `End` DateTime CODEC(Delta(4), ZSTD(1)),
    INDEX idx_trace_id TraceId TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Start)
ORDER BY (OrgId, TraceId, Start)
TTL toDate(Start) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_ci_traces_trace_id_ts cityHash64(TraceId)};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.otel_ci_traces_trace_id_ts_mv ${ON_CLUSTER} TO otel.otel_ci_traces_trace_id_ts${LOCAL}
(
    `OrgId` LowCardinality(String),
    `TraceId` String,
    `Start` DateTime64(9),
    `End` DateTime64(9)
)
AS SELECT
    ResourceAttributes['oc.org_id'] AS OrgId,
    TraceId,
    min(Timestamp) AS Start,
    max(Timestamp) AS End
FROM otel.otel_ci_traces${LOCAL}
WHERE TraceId != ''
GROUP BY OrgId, TraceId;

-- ================================================================ metrics
-- One table per OTel point type. Sorted by org, then metric, so a query reads one org's series of
-- one metric; sharded by series.

CREATE TABLE IF NOT EXISTS otel.otel_metrics_gauge${LOCAL} ${ON_CLUSTER}
(

    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ResourceSchemaUrl` String CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeDroppedAttrCount` UInt32 CODEC(ZSTD(1)),
    `ScopeSchemaUrl` String CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `MetricName` String CODEC(ZSTD(1)),
    `MetricDescription` String CODEC(ZSTD(1)),
    `MetricUnit` String CODEC(ZSTD(1)),
    `Attributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `StartTimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `Value` Float64 CODEC(ZSTD(1)),
    `Flags` UInt32 CODEC(ZSTD(1)),
    `Exemplars.FilteredAttributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `Exemplars.TimeUnix` Array(DateTime64(9)) CODEC(ZSTD(1)),
    `Exemplars.Value` Array(Float64) CODEC(ZSTD(1)),
    `Exemplars.SpanId` Array(String) CODEC(ZSTD(1)),
    `Exemplars.TraceId` Array(String) CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_key mapKeys(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_value mapValues(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimeUnix)
ORDER BY (OrgId, MetricName, ServiceName, Attributes, toUnixTimestamp64Nano(TimeUnix))
TTL toDateTime(TimeUnix) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_metrics_gauge cityHash64(ResourceAttributes['oc.org_id'], MetricName, ServiceName, toString(Attributes), toString(ResourceAttributes))};

CREATE TABLE IF NOT EXISTS otel.otel_metrics_sum${LOCAL} ${ON_CLUSTER}
(

    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ResourceSchemaUrl` String CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeDroppedAttrCount` UInt32 CODEC(ZSTD(1)),
    `ScopeSchemaUrl` String CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `MetricName` String CODEC(ZSTD(1)),
    `MetricDescription` String CODEC(ZSTD(1)),
    `MetricUnit` String CODEC(ZSTD(1)),
    `Attributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `StartTimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `Value` Float64 CODEC(ZSTD(1)),
    `Flags` UInt32 CODEC(ZSTD(1)),
    `Exemplars.FilteredAttributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `Exemplars.TimeUnix` Array(DateTime64(9)) CODEC(ZSTD(1)),
    `Exemplars.Value` Array(Float64) CODEC(ZSTD(1)),
    `Exemplars.SpanId` Array(String) CODEC(ZSTD(1)),
    `Exemplars.TraceId` Array(String) CODEC(ZSTD(1)),
    `AggregationTemporality` Int32 CODEC(ZSTD(1)),
    `IsMonotonic` Bool CODEC(Delta(1), ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_key mapKeys(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_value mapValues(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimeUnix)
ORDER BY (OrgId, MetricName, ServiceName, Attributes, toUnixTimestamp64Nano(TimeUnix))
TTL toDateTime(TimeUnix) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_metrics_sum cityHash64(ResourceAttributes['oc.org_id'], MetricName, ServiceName, toString(Attributes), toString(ResourceAttributes))};

CREATE TABLE IF NOT EXISTS otel.otel_metrics_histogram${LOCAL} ${ON_CLUSTER}
(

    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ResourceSchemaUrl` String CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeDroppedAttrCount` UInt32 CODEC(ZSTD(1)),
    `ScopeSchemaUrl` String CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `MetricName` String CODEC(ZSTD(1)),
    `MetricDescription` String CODEC(ZSTD(1)),
    `MetricUnit` String CODEC(ZSTD(1)),
    `Attributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `StartTimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `Count` UInt64 CODEC(Delta(8), ZSTD(1)),
    `Sum` Float64 CODEC(ZSTD(1)),
    `BucketCounts` Array(UInt64) CODEC(ZSTD(1)),
    `ExplicitBounds` Array(Float64) CODEC(ZSTD(1)),
    `Exemplars.FilteredAttributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `Exemplars.TimeUnix` Array(DateTime64(9)) CODEC(ZSTD(1)),
    `Exemplars.Value` Array(Float64) CODEC(ZSTD(1)),
    `Exemplars.SpanId` Array(String) CODEC(ZSTD(1)),
    `Exemplars.TraceId` Array(String) CODEC(ZSTD(1)),
    `Flags` UInt32 CODEC(ZSTD(1)),
    `Min` Float64 CODEC(ZSTD(1)),
    `Max` Float64 CODEC(ZSTD(1)),
    `AggregationTemporality` Int32 CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_key mapKeys(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_value mapValues(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimeUnix)
ORDER BY (OrgId, MetricName, ServiceName, Attributes, toUnixTimestamp64Nano(TimeUnix))
TTL toDateTime(TimeUnix) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_metrics_histogram cityHash64(ResourceAttributes['oc.org_id'], MetricName, ServiceName, toString(Attributes), toString(ResourceAttributes))};

CREATE TABLE IF NOT EXISTS otel.otel_metrics_exponential_histogram${LOCAL} ${ON_CLUSTER}
(

    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ResourceSchemaUrl` String CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeDroppedAttrCount` UInt32 CODEC(ZSTD(1)),
    `ScopeSchemaUrl` String CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `MetricName` String CODEC(ZSTD(1)),
    `MetricDescription` String CODEC(ZSTD(1)),
    `MetricUnit` String CODEC(ZSTD(1)),
    `Attributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `StartTimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `Count` UInt64 CODEC(Delta(8), ZSTD(1)),
    `Sum` Float64 CODEC(ZSTD(1)),
    `Scale` Int32 CODEC(ZSTD(1)),
    `ZeroCount` UInt64 CODEC(ZSTD(1)),
    `PositiveOffset` Int32 CODEC(ZSTD(1)),
    `PositiveBucketCounts` Array(UInt64) CODEC(ZSTD(1)),
    `NegativeOffset` Int32 CODEC(ZSTD(1)),
    `NegativeBucketCounts` Array(UInt64) CODEC(ZSTD(1)),
    `Exemplars.FilteredAttributes` Array(Map(LowCardinality(String), String)) CODEC(ZSTD(1)),
    `Exemplars.TimeUnix` Array(DateTime64(9)) CODEC(ZSTD(1)),
    `Exemplars.Value` Array(Float64) CODEC(ZSTD(1)),
    `Exemplars.SpanId` Array(String) CODEC(ZSTD(1)),
    `Exemplars.TraceId` Array(String) CODEC(ZSTD(1)),
    `Flags` UInt32 CODEC(ZSTD(1)),
    `Min` Float64 CODEC(ZSTD(1)),
    `Max` Float64 CODEC(ZSTD(1)),
    `AggregationTemporality` Int32 CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_key mapKeys(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_value mapValues(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimeUnix)
ORDER BY (OrgId, MetricName, ServiceName, Attributes, toUnixTimestamp64Nano(TimeUnix))
TTL toDateTime(TimeUnix) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_metrics_exponential_histogram cityHash64(ResourceAttributes['oc.org_id'], MetricName, ServiceName, toString(Attributes), toString(ResourceAttributes))};

CREATE TABLE IF NOT EXISTS otel.otel_metrics_summary${LOCAL} ${ON_CLUSTER}
(

    `ResourceAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ResourceSchemaUrl` String CODEC(ZSTD(1)),
    `ScopeName` String CODEC(ZSTD(1)),
    `ScopeVersion` String CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `ScopeDroppedAttrCount` UInt32 CODEC(ZSTD(1)),
    `ScopeSchemaUrl` String CODEC(ZSTD(1)),
    `ServiceName` LowCardinality(String) CODEC(ZSTD(1)),
    `MetricName` String CODEC(ZSTD(1)),
    `MetricDescription` String CODEC(ZSTD(1)),
    `MetricUnit` String CODEC(ZSTD(1)),
    `Attributes` Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    `StartTimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `TimeUnix` DateTime64(9) CODEC(Delta(8), ZSTD(1)),
    `Count` UInt64 CODEC(Delta(8), ZSTD(1)),
    `Sum` Float64 CODEC(ZSTD(1)),
    `ValueAtQuantiles.Quantile` Array(Float64) CODEC(ZSTD(1)),
    `ValueAtQuantiles.Value` Array(Float64) CODEC(ZSTD(1)),
    `Flags` UInt32 CODEC(ZSTD(1)),
    `OrgId` LowCardinality(String) MATERIALIZED ResourceAttributes['oc.org_id'],
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_key mapKeys(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_value mapValues(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(TimeUnix)
ORDER BY (OrgId, MetricName, ServiceName, Attributes, toUnixTimestamp64Nano(TimeUnix))
TTL toDateTime(TimeUnix) + toIntervalDay(30)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_metrics_summary cityHash64(ResourceAttributes['oc.org_id'], MetricName, ServiceName, toString(Attributes), toString(ResourceAttributes))};

-- ================================================================ Kubernetes
-- The cluster collector's k8sobjects receiver sends every object each minute and events as they
-- happen (resource oc.signal = k8s.objects). Materialized views keep the latest state of each object
-- and the events; objects are sharded by uid so an object's versions dedupe on one shard.
CREATE TABLE IF NOT EXISTS otel.otel_k8s_objects${LOCAL} ${ON_CLUSTER}
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
TTL TimestampTime + INTERVAL 1 DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
${DISTRIBUTED otel.otel_k8s_objects cityHash64(ResourceAttributes['oc.org_id'], JSONExtractString(Body, 'metadata', 'uid'), JSONExtractString(Body, 'object', 'metadata', 'uid'))};

CREATE TABLE IF NOT EXISTS otel.k8s_resources${LOCAL} ${ON_CLUSTER}
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
ENGINE = ${REPLICATED}ReplacingMergeTree(LastSeen)
ORDER BY (OrgId, Cluster, Kind, Namespace, Name, Uid)
TTL toDateTime(LastSeen) + INTERVAL 7 DAY;
${DISTRIBUTED otel.k8s_resources cityHash64(OrgId, Uid)};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.k8s_resources_mv ${ON_CLUSTER} TO otel.k8s_resources${LOCAL} AS
SELECT
    ResourceAttributes['oc.org_id'] AS OrgId,
    ResourceAttributes['k8s.cluster.name'] AS Cluster,
    JSONExtractString(Body, 'kind') AS Kind,
    JSONExtractString(Body, 'metadata', 'namespace') AS Namespace,
    JSONExtractString(Body, 'metadata', 'name') AS Name,
    JSONExtractString(Body, 'metadata', 'uid') AS Uid,
    Body,
    now64(3) AS LastSeen
FROM otel.otel_k8s_objects${LOCAL}
WHERE JSONExtractString(Body, 'kind') NOT IN ('', 'Event') AND JSONExtractString(Body, 'metadata', 'uid') != '';

CREATE TABLE IF NOT EXISTS otel.k8s_events${LOCAL} ${ON_CLUSTER}
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
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Timestamp)
ORDER BY (OrgId, Cluster, RegardingUid, Timestamp)
TTL toDateTime(Timestamp) + INTERVAL 7 DAY;
${DISTRIBUTED otel.k8s_events cityHash64(OrgId, RegardingUid)};

-- Watch-mode records are {"type": "ADDED" | "MODIFIED" | …, "object": <events.k8s.io/v1 Event>}.
-- (The inner query keeps the source Body from being shadowed by the output column of the same name.)
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.k8s_events_mv ${ON_CLUSTER} TO otel.k8s_events${LOCAL} AS
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
    FROM otel.otel_k8s_objects${LOCAL}
    WHERE JSONExtractString(Body, 'object', 'kind') = 'Event'
);
