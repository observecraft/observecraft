-- Change tracking (Infrastructure → Changes) compares successive generations of deployments, stateful
-- sets and daemon sets. Reading them from the raw object snapshots meant parsing every object's JSON
-- (the collectors resend each object every minute); this table keeps one row per generation instead,
-- with only what the comparison needs:
--   Replicas            spec.replicas (NULL when unset, as on daemon sets)
--   Images              spec.template.spec.containers[*].image, in container order
--   TemplateHash        hash of spec.template
--   TemplateSpecHashes  hash of each key of spec.template.spec
--   SpecHashes          hash of each other key of spec (all but replicas and template)
--   Labels              the labels service and env come from
-- Rows of one generation from different inserts merge (FirstSeen the earliest snapshot, LastSeen the
-- latest); readers still GROUP BY. A row lives 30 days after its generation was last seen, so the
-- current generation of a workload stays the baseline of its next change however old it is.
CREATE TABLE IF NOT EXISTS otel.k8s_workload_generations${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Uid` String,
    `Generation` Int64,
    `Cluster` SimpleAggregateFunction(any, LowCardinality(String)),
    `Kind` SimpleAggregateFunction(any, LowCardinality(String)),
    `Namespace` SimpleAggregateFunction(any, String),
    `Name` SimpleAggregateFunction(any, String),
    `Replicas` SimpleAggregateFunction(any, Nullable(Int64)),
    `Images` SimpleAggregateFunction(any, Array(String)),
    `TemplateHash` SimpleAggregateFunction(any, UInt64),
    `TemplateSpecHashes` SimpleAggregateFunction(any, Map(String, UInt64)),
    `SpecHashes` SimpleAggregateFunction(any, Map(String, UInt64)),
    `Labels` SimpleAggregateFunction(any, Map(String, String)),
    `FirstSeen` SimpleAggregateFunction(min, DateTime64(3)),
    `LastSeen` SimpleAggregateFunction(max, DateTime64(3))
)
ENGINE = ${REPLICATED}AggregatingMergeTree
ORDER BY (OrgId, Uid, Generation)
TTL toDateTime(LastSeen) + INTERVAL 30 DAY;
${DISTRIBUTED otel.k8s_workload_generations cityHash64(OrgId, Uid)};

-- Objects are sharded by uid, so all snapshots of a workload reach the same shard's view. Each insert
-- block is pre-aggregated to one row per generation.
CREATE MATERIALIZED VIEW IF NOT EXISTS otel.k8s_workload_generations_mv ${ON_CLUSTER} TO otel.k8s_workload_generations${LOCAL} AS
SELECT
    org AS OrgId, uid AS Uid, generation AS Generation,
    any(cluster) AS Cluster, any(kind) AS Kind, any(namespace) AS Namespace, any(name) AS Name,
    any(replicas) AS Replicas,
    any(arrayMap(c -> JSONExtractString(c, 'image'), JSONExtractArrayRaw(spec, 'template', 'spec', 'containers'))) AS Images,
    any(cityHash64(JSONExtractRaw(spec, 'template'))) AS TemplateHash,
    any(mapFromArrays(arrayMap(kv -> kv.1, JSONExtractKeysAndValuesRaw(spec, 'template', 'spec')), arrayMap(kv -> cityHash64(kv.2), JSONExtractKeysAndValuesRaw(spec, 'template', 'spec')))) AS TemplateSpecHashes,
    any(mapFilter((k, v) -> k NOT IN ('replicas', 'template'), mapFromArrays(arrayMap(kv -> kv.1, JSONExtractKeysAndValuesRaw(spec)), arrayMap(kv -> cityHash64(kv.2), JSONExtractKeysAndValuesRaw(spec))))) AS SpecHashes,
    any(mapFilter((k, v) -> k IN ('tags.datadoghq.com/service', 'tags.datadoghq.com/env', 'app.kubernetes.io/name', 'app'), labels)) AS Labels,
    min(seen) AS FirstSeen,
    max(seen) AS LastSeen
FROM
(
    SELECT
        ResourceAttributes['oc.org_id'] AS org,
        ResourceAttributes['k8s.cluster.name'] AS cluster,
        JSONExtractString(Body, 'kind') AS kind,
        JSONExtractString(Body, 'metadata', 'namespace') AS namespace,
        JSONExtractString(Body, 'metadata', 'name') AS name,
        JSONExtractString(Body, 'metadata', 'uid') AS uid,
        JSONExtractInt(Body, 'metadata', 'generation') AS generation,
        JSONExtractRaw(Body, 'spec') AS spec,
        if(JSONHas(spec, 'replicas'), JSONExtractInt(spec, 'replicas'), NULL) AS replicas,
        JSONExtract(Body, 'metadata', 'labels', 'Map(String, String)') AS labels,
        toDateTime64(Timestamp, 3) AS seen
    FROM otel.otel_k8s_objects${LOCAL}
    WHERE JSONExtractString(Body, 'kind') IN ('Deployment', 'StatefulSet', 'DaemonSet') AND JSONExtractString(Body, 'metadata', 'uid') != ''
)
GROUP BY org, uid, generation;

-- Backfill from the snapshots still in the raw table (it keeps a day), once.
INSERT INTO otel.k8s_workload_generations
SELECT
    org AS OrgId, uid AS Uid, generation AS Generation,
    any(cluster) AS Cluster, any(kind) AS Kind, any(namespace) AS Namespace, any(name) AS Name,
    any(replicas) AS Replicas,
    any(arrayMap(c -> JSONExtractString(c, 'image'), JSONExtractArrayRaw(spec, 'template', 'spec', 'containers'))) AS Images,
    any(cityHash64(JSONExtractRaw(spec, 'template'))) AS TemplateHash,
    any(mapFromArrays(arrayMap(kv -> kv.1, JSONExtractKeysAndValuesRaw(spec, 'template', 'spec')), arrayMap(kv -> cityHash64(kv.2), JSONExtractKeysAndValuesRaw(spec, 'template', 'spec')))) AS TemplateSpecHashes,
    any(mapFilter((k, v) -> k NOT IN ('replicas', 'template'), mapFromArrays(arrayMap(kv -> kv.1, JSONExtractKeysAndValuesRaw(spec)), arrayMap(kv -> cityHash64(kv.2), JSONExtractKeysAndValuesRaw(spec))))) AS SpecHashes,
    any(mapFilter((k, v) -> k IN ('tags.datadoghq.com/service', 'tags.datadoghq.com/env', 'app.kubernetes.io/name', 'app'), labels)) AS Labels,
    min(seen) AS FirstSeen,
    max(seen) AS LastSeen
FROM
(
    SELECT
        OrgId AS org,
        ResourceAttributes['k8s.cluster.name'] AS cluster,
        JSONExtractString(Body, 'kind') AS kind,
        JSONExtractString(Body, 'metadata', 'namespace') AS namespace,
        JSONExtractString(Body, 'metadata', 'name') AS name,
        JSONExtractString(Body, 'metadata', 'uid') AS uid,
        JSONExtractInt(Body, 'metadata', 'generation') AS generation,
        JSONExtractRaw(Body, 'spec') AS spec,
        if(JSONHas(spec, 'replicas'), JSONExtractInt(spec, 'replicas'), NULL) AS replicas,
        JSONExtract(Body, 'metadata', 'labels', 'Map(String, String)') AS labels,
        toDateTime64(Timestamp, 3) AS seen
    FROM otel.otel_k8s_objects
    WHERE JSONExtractString(Body, 'kind') IN ('Deployment', 'StatefulSet', 'DaemonSet') AND JSONExtractString(Body, 'metadata', 'uid') != ''
)
GROUP BY org, uid, generation;
