-- Infrastructure → Resources → Changes: one row per detected change of a resource's configuration
-- (created, deleted, or fields changed), written by the server's change job from the catalog's
-- snapshots (routes/resource_changes.rs). OldFields / NewFields are the configuration before and
-- after (JSON, '' when the resource didn't exist). Kept 30 days.
CREATE TABLE IF NOT EXISTS otel.resource_changes${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Detected` DateTime64(3, 'UTC'),
    `ResourceId` String,
    `ResourceType` LowCardinality(String),
    `ResourceName` String,
    `Cloud` LowCardinality(String),
    `Account` LowCardinality(String),
    `Region` LowCardinality(String),
    `Cluster` LowCardinality(String),
    `Namespace` LowCardinality(String),
    `Env` LowCardinality(String),
    `Service` LowCardinality(String),
    `Team` LowCardinality(String),
    `ChangeTypes` Array(LowCardinality(String)),
    `ChangedFields` Array(String),
    `Risk` LowCardinality(String),
    `Title` String,
    `Description` String,
    `IsNoise` Bool,
    `OldFields` String CODEC(ZSTD(3)),
    `NewFields` String CODEC(ZSTD(3))
)
ENGINE = ${REPLICATED}MergeTree
PARTITION BY toDate(Detected)
ORDER BY (OrgId, Detected, ResourceId)
TTL toDateTime(Detected) + INTERVAL 30 DAY;
${DISTRIBUTED otel.resource_changes cityHash64(OrgId, ResourceId)};
