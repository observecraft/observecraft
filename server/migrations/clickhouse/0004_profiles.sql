-- Continuous profiles (Datadog's profiler, through the Agent's `apm_config.profiling_dd_url` and the
-- intake): one row per uploaded pprof file, gzip-compressed as sent. Written by the server's
-- internal listener (the intake forwards each upload with its organization). Kept 8 days, like
-- Datadog's profiles.
CREATE TABLE IF NOT EXISTS otel.profiles${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Start` DateTime64(9),
    `End` DateTime64(9),
    `Service` LowCardinality(String),
    `Env` LowCardinality(String),
    `Version` String,
    `Host` String,
    -- python, go, java, ruby, node, dotnet, php, ddprof
    `Language` LowCardinality(String),
    `RuntimeId` String,
    -- The attachment's file name: auto.pprof, cpu.pprof, heap.pprof, …
    `Attachment` LowCardinality(String),
    `Tags` Map(LowCardinality(String), String),
    `Pprof` String CODEC(ZSTD(1))
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Start)
ORDER BY (OrgId, Service, Start)
TTL toDateTime(Start) + INTERVAL 8 DAY
SETTINGS ttl_only_drop_parts = 1;
${DISTRIBUTED otel.profiles rand()};
