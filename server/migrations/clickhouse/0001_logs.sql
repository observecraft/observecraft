-- Logs written by the Vector aggregator (deploy/vector/aggregator.yaml → clickhouse sink).
-- Applied by the API on startup (oc-store) and by the ClickHouse container on first boot,
-- so every statement must stay idempotent. Statements are split on ';'.
CREATE DATABASE IF NOT EXISTS oc;

CREATE TABLE IF NOT EXISTS oc.logs
(
    org_id      LowCardinality(String),
    id          UUID DEFAULT generateUUIDv4(),
    timestamp   DateTime64(3, 'UTC') CODEC(Delta(8), ZSTD(1)),
    status      LowCardinality(String),
    service     LowCardinality(String),
    host        LowCardinality(String),
    source      LowCardinality(String),
    message     String CODEC(ZSTD(1)),
    -- Infrastructure tags (env, kube_namespace, pod_name, …) — queried as key:value.
    tags        Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    -- Flattened event attributes (http.status_code, …) — queried as @path:value.
    attr        Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    -- Original attributes as JSON, for display.
    attributes  String CODEC(ZSTD(3)),

    INDEX idx_message lower(message) TYPE ngrambf_v1(3, 65536, 3, 0) GRANULARITY 1,
    INDEX idx_tag_keys mapKeys(tags) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_tag_values mapValues(tags) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_keys mapKeys(attr) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_values mapValues(attr) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_host host TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = MergeTree
PARTITION BY toDate(timestamp)
ORDER BY (org_id, service, timestamp)
TTL toDateTime(timestamp) + INTERVAL 15 DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;
