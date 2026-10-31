-- AI Guard evaluations (`POST /api/v2/ai-guard/evaluate`): one row per evaluated step of an LLM
-- conversation, written by the API server's evaluator. Messages keep the conversation as sent
-- (JSON), Conversation groups the steps of one run (same service and opening messages). Read by
-- AI Guard's Investigate, Discover and detection rules. Kept 30 days.
CREATE TABLE IF NOT EXISTS otel.ai_guard_evaluations${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Timestamp` DateTime64(3, 'UTC') CODEC(Delta(8), ZSTD(1)),
    `Id` String CODEC(ZSTD(1)),
    `Conversation` String CODEC(ZSTD(1)),
    `Service` LowCardinality(String),
    `Env` LowCardinality(String),
    `Target` LowCardinality(String),
    `Direction` LowCardinality(String),
    `ToolName` LowCardinality(String),
    `Action` LowCardinality(String),
    `Blocked` UInt8,
    `BlockingEnabled` UInt8,
    `Reason` String CODEC(ZSTD(1)),
    `Tags` Array(LowCardinality(String)),
    `TagNames` Array(LowCardinality(String)),
    `TagProbs` Array(Float64),
    `SdsTags` Array(LowCardinality(String)),
    `SdsCategories` Array(LowCardinality(String)),
    `Sds` String CODEC(ZSTD(3)),
    `Content` String CODEC(ZSTD(3)),
    `Messages` String CODEC(ZSTD(3)),
    `DurationMs` Float64,
    `Source` LowCardinality(String),
    `Language` LowCardinality(String),
    `ClientIp` String CODEC(ZSTD(1)),
    `UserId` String CODEC(ZSTD(1)),
    `UserEmail` String CODEC(ZSTD(1)),
    `SessionId` String CODEC(ZSTD(1)),
    `TraceId` String CODEC(ZSTD(1)),
    INDEX idx_conversation Conversation TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_id Id TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Timestamp)
ORDER BY (OrgId, Service, Timestamp)
TTL toDateTime(Timestamp) + INTERVAL 30 DAY
SETTINGS ttl_only_drop_parts = 1;
${DISTRIBUTED otel.ai_guard_evaluations rand()};
