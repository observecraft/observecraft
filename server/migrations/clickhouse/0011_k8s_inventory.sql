-- The Kubernetes Explorer's inventory as typed columns. Explorers, dashboards and the resource
-- catalog read the current state of every object of an org; reading it from k8s_resources meant
-- reading each object's whole JSON (managedFields, last-applied configuration, pod specs with their
-- env, CRD schemas, node image lists) and parsing it on the server. This table keeps, per object:
--   typed columns     what explorers filter, sort and facet on (namespace, phase, node, owner,
--                     ready / total containers, restarts, created, labels, CPU and memory requests
--                     and limits, pod IP) and the object's annotations
--   Doc               a trimmed JSON copy of the parts of spec and status the explorer's columns are
--                     built from (container names, images, resources and states; replica counts;
--                     service ports; node info; …), a small fraction of the object
-- k8s_resources keeps the full object for the detail panel (read by uid). Like it, a deleted object
-- stays 7 days (network flows of the past week still resolve pod and service addresses).
--
-- Quantities (`250m`, `64Mi`, `1G`, `129e6`) are cores and bytes; a pod's CPU / memory requests are
-- the sum over its regular containers (NULL when none sets one), its limits only when every regular
-- container sets one, as the explorer computes them.
--
-- No backfill: the collectors resend every object at least every few minutes (one not seen for 3
-- minutes is gone from the explorers), so the table is complete minutes after the upgrade.
CREATE TABLE IF NOT EXISTS otel.k8s_inventory${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `Cluster` LowCardinality(String),
    `Kind` LowCardinality(String),
    `Uid` String,
    `Namespace` LowCardinality(String),
    `Name` String,
    `Created` Nullable(DateTime('UTC')),
    `Deleting` Bool,
    `Labels` Map(LowCardinality(String), String),
    `Annotations` Map(LowCardinality(String), String),
    `OwnerKind` LowCardinality(String),
    `OwnerName` String,
    `OwnerUid` String,
    `Phase` LowCardinality(String),
    `Node` LowCardinality(String),
    `Ip` String,
    `HostNetwork` Bool,
    `ReadyContainers` UInt16,
    `Containers` UInt16,
    `Restarts` UInt32,
    `CpuRequests` Nullable(Float64),
    `CpuLimits` Nullable(Float64),
    `MemoryRequests` Nullable(Float64),
    `MemoryLimits` Nullable(Float64),
    `Doc` String CODEC(ZSTD(3)),
    `LastSeen` DateTime64(3)
)
ENGINE = ${REPLICATED}ReplacingMergeTree(LastSeen)
ORDER BY (OrgId, Kind, Uid)
TTL toDateTime(LastSeen) + INTERVAL 7 DAY;
-- Sharded like otel_k8s_objects (org, uid and the empty event uid): a direct insert lands on the
-- shard whose view writes the object's snapshots.
${DISTRIBUTED otel.k8s_inventory cityHash64(OrgId, Uid, '')};

CREATE MATERIALIZED VIEW IF NOT EXISTS otel.k8s_inventory_mv ${ON_CLUSTER} TO otel.k8s_inventory${LOCAL} AS
WITH
    -- A raw JSON value, or null when the key is missing.
    raw -> if(raw = '', 'null', raw) AS orNull,
    -- Kubernetes quantity → number (cores, bytes); NULL when unset or not a quantity.
    q -> toFloat64OrNull(extract(trimBoth(q), '^[^a-df-zA-DF-Z]*')) * multiIf(
        substring(trimBoth(q), length(extract(trimBoth(q), '^[^a-df-zA-DF-Z]*')) + 1) AS unit = '', 1,
        unit = 'm', 1e-3, unit = 'u', 1e-6, unit = 'n', 1e-9, unit = 'k', 1e3, unit = 'M', 1e6, unit = 'G', 1e9, unit = 'T', 1e12, unit = 'P', 1e15,
        unit = 'Ki', 1024, unit = 'Mi', pow(1024, 2), unit = 'Gi', pow(1024, 3), unit = 'Ti', pow(1024, 4), unit = 'Pi', pow(1024, 5), NULL) AS quantity,
    -- Containers: name, image and resources only (not env, probes, mounts, …).
    cs -> concat('[', arrayStringConcat(arrayMap(c -> concat('{"name":', orNull(JSONExtractRaw(c, 'name')), ',"image":', orNull(JSONExtractRaw(c, 'image')), ',"resources":', orNull(JSONExtractRaw(c, 'resources')), '}'), cs), ','), ']') AS containersDoc,
    -- Container statuses: name, readiness, restarts and state.
    ss -> concat('[', arrayStringConcat(arrayMap(c -> concat('{"name":', orNull(JSONExtractRaw(c, 'name')), ',"ready":', orNull(JSONExtractRaw(c, 'ready')), ',"restartCount":', orNull(JSONExtractRaw(c, 'restartCount')), ',"state":', orNull(JSONExtractRaw(c, 'state')), '}'), ss), ','), ']') AS statusesDoc,
    -- A pod template's containers: their images.
    cs -> concat('[', arrayStringConcat(arrayMap(c -> concat('{"image":', orNull(JSONExtractRaw(c, 'image')), '}'), cs), ','), ']') AS imagesDoc,
    -- The controller, else the first owner.
    if(arrayExists(r -> JSONExtractBool(r, 'controller'), refs), arrayFirst(r -> JSONExtractBool(r, 'controller'), refs), refs[1]) AS owner,
    arrayMap(c -> quantity(JSONExtractString(c, 'resources', 'requests', 'cpu')), containers) AS cpuRequests,
    arrayMap(c -> quantity(JSONExtractString(c, 'resources', 'limits', 'cpu')), containers) AS cpuLimits,
    arrayMap(c -> quantity(JSONExtractString(c, 'resources', 'requests', 'memory')), containers) AS memoryRequests,
    arrayMap(c -> quantity(JSONExtractString(c, 'resources', 'limits', 'memory')), containers) AS memoryLimits
SELECT
    OrgId, Cluster, Kind, Uid, Namespace, Name,
    parseDateTimeBestEffortOrNull(JSONExtractString(Body, 'metadata', 'creationTimestamp'), 'UTC') AS Created,
    JSONExtractString(Body, 'metadata', 'deletionTimestamp') != '' AS Deleting,
    JSONExtract(Body, 'metadata', 'labels', 'Map(String, String)') AS Labels,
    mapFilter((k, v) -> k != 'kubectl.kubernetes.io/last-applied-configuration', JSONExtract(Body, 'metadata', 'annotations', 'Map(String, String)')) AS Annotations,
    JSONExtractString(owner, 'kind') AS OwnerKind,
    JSONExtractString(owner, 'name') AS OwnerName,
    JSONExtractString(owner, 'uid') AS OwnerUid,
    JSONExtractString(Body, 'status', 'phase') AS Phase,
    JSONExtractString(Body, 'spec', 'nodeName') AS Node,
    multiIf(Kind = 'Pod', JSONExtractString(Body, 'status', 'podIP'), Kind = 'Service', JSONExtractString(Body, 'spec', 'clusterIP'), '') AS Ip,
    JSONExtractBool(Body, 'spec', 'hostNetwork') AS HostNetwork,
    toUInt16(if(Kind = 'Pod', arrayCount(c -> JSONExtractBool(arrayFirst(s -> JSONExtractString(s, 'name') = JSONExtractString(c, 'name'), statuses), 'ready'), containers), 0)) AS ReadyContainers,
    toUInt16(if(Kind = 'Pod', length(containers), 0)) AS Containers,
    toUInt32(if(Kind = 'Pod', arraySum(arrayMap(c -> JSONExtractUInt(arrayFirst(s -> JSONExtractString(s, 'name') = JSONExtractString(c, 'name'), arrayConcat(initStatuses, statuses)), 'restartCount'), arrayConcat(initContainers, containers))), 0)) AS Restarts,
    if(Kind = 'Pod' AND arrayExists(x -> isNotNull(x), cpuRequests), arraySum(x -> ifNull(x, 0), cpuRequests), NULL) AS CpuRequests,
    if(Kind = 'Pod' AND notEmpty(cpuLimits) AND arrayAll(x -> isNotNull(x), cpuLimits), arraySum(x -> ifNull(x, 0), cpuLimits), NULL) AS CpuLimits,
    if(Kind = 'Pod' AND arrayExists(x -> isNotNull(x), memoryRequests), arraySum(x -> ifNull(x, 0), memoryRequests), NULL) AS MemoryRequests,
    if(Kind = 'Pod' AND notEmpty(memoryLimits) AND arrayAll(x -> isNotNull(x), memoryLimits), arraySum(x -> ifNull(x, 0), memoryLimits), NULL) AS MemoryLimits,
    multiIf(
        Kind = 'Pod',
        concat('{"spec":{"nodeName":', orNull(JSONExtractRaw(Body, 'spec', 'nodeName')),
               ',"initContainers":', containersDoc(initContainers), ',"containers":', containersDoc(containers),
               '},"status":{"phase":', orNull(JSONExtractRaw(Body, 'status', 'phase')), ',"podIP":', orNull(JSONExtractRaw(Body, 'status', 'podIP')),
               ',"qosClass":', orNull(JSONExtractRaw(Body, 'status', 'qosClass')), ',"conditions":', orNull(JSONExtractRaw(Body, 'status', 'conditions')),
               ',"initContainerStatuses":', statusesDoc(initStatuses), ',"containerStatuses":', statusesDoc(statuses), '}}'),
        Kind IN ('Deployment', 'StatefulSet', 'ReplicaSet', 'DaemonSet', 'Job'),
        concat('{"spec":{"replicas":', orNull(JSONExtractRaw(Body, 'spec', 'replicas')), ',"completions":', orNull(JSONExtractRaw(Body, 'spec', 'completions')),
               ',"suspend":', orNull(JSONExtractRaw(Body, 'spec', 'suspend')), ',"selector":', orNull(JSONExtractRaw(Body, 'spec', 'selector')),
               ',"template":{"spec":{"containers":', imagesDoc(JSONExtractArrayRaw(Body, 'spec', 'template', 'spec', 'containers')), '}}},"status":', orNull(JSONExtractRaw(Body, 'status')), '}'),
        Kind = 'CronJob',
        concat('{"spec":{"schedule":', orNull(JSONExtractRaw(Body, 'spec', 'schedule')), ',"suspend":', orNull(JSONExtractRaw(Body, 'spec', 'suspend')),
               ',"jobTemplate":{"spec":{"template":{"spec":{"containers":', imagesDoc(JSONExtractArrayRaw(Body, 'spec', 'jobTemplate', 'spec', 'template', 'spec', 'containers')),
               '}}}}},"status":', orNull(JSONExtractRaw(Body, 'status')), '}'),
        Kind = 'Node',
        concat('{"spec":{"unschedulable":', orNull(JSONExtractRaw(Body, 'spec', 'unschedulable')), '},"status":{"conditions":', orNull(JSONExtractRaw(Body, 'status', 'conditions')),
               ',"nodeInfo":', orNull(JSONExtractRaw(Body, 'status', 'nodeInfo')), ',"addresses":', orNull(JSONExtractRaw(Body, 'status', 'addresses')),
               ',"allocatable":', orNull(JSONExtractRaw(Body, 'status', 'allocatable')), '}}'),
        Kind = 'CustomResourceDefinition',
        concat('{"spec":{"group":', orNull(JSONExtractRaw(Body, 'spec', 'group')), ',"scope":', orNull(JSONExtractRaw(Body, 'spec', 'scope')),
               ',"versions":[', arrayStringConcat(arrayMap(v -> concat('{"name":', orNull(JSONExtractRaw(v, 'name')), ',"storage":', orNull(JSONExtractRaw(v, 'storage')), '}'), JSONExtractArrayRaw(Body, 'spec', 'versions')), ','), ']}}'),
        -- Roles: only the number of rules is shown.
        Kind IN ('Role', 'ClusterRole'),
        concat('{"rules":[', arrayStringConcat(arrayMap(r -> '{}', JSONExtractArrayRaw(Body, 'rules')), ','), ']}'),
        -- Everything else is small: services, ingresses, volumes, autoscalers, bindings, …
        concat('{"spec":', orNull(JSONExtractRaw(Body, 'spec')), ',"status":', orNull(JSONExtractRaw(Body, 'status')),
               ',"roleRef":', orNull(JSONExtractRaw(Body, 'roleRef')), ',"subjects":', orNull(JSONExtractRaw(Body, 'subjects')),
               ',"provisioner":', orNull(JSONExtractRaw(Body, 'provisioner')), ',"reclaimPolicy":', orNull(JSONExtractRaw(Body, 'reclaimPolicy')), '}')
    ) AS Doc,
    now64(3) AS LastSeen
FROM
(
    SELECT
        ResourceAttributes['oc.org_id'] AS OrgId,
        ResourceAttributes['k8s.cluster.name'] AS Cluster,
        JSONExtractString(Body, 'kind') AS Kind,
        JSONExtractString(Body, 'metadata', 'uid') AS Uid,
        JSONExtractString(Body, 'metadata', 'namespace') AS Namespace,
        JSONExtractString(Body, 'metadata', 'name') AS Name,
        JSONExtractArrayRaw(Body, 'metadata', 'ownerReferences') AS refs,
        JSONExtractArrayRaw(Body, 'spec', 'containers') AS containers,
        JSONExtractArrayRaw(Body, 'spec', 'initContainers') AS initContainers,
        JSONExtractArrayRaw(Body, 'status', 'containerStatuses') AS statuses,
        JSONExtractArrayRaw(Body, 'status', 'initContainerStatuses') AS initStatuses,
        Body
    FROM otel.otel_k8s_objects${LOCAL}
    WHERE Kind NOT IN ('', 'Event') AND Uid != ''
);
